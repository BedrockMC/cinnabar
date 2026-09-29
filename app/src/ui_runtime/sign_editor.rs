//! Sign editor state: four lines of formatted text with a cursor, edited in place and sent back
//! as the whole sign compound.

mod input;

pub(crate) use input::drive_sign_editor;
use world::{NbtCompound, NbtValue};

pub(crate) const SIGN_LINES: usize = 4;
/// Widest a line may measure, in design pixels; longer input is refused.
pub(crate) const MAX_LINE_DESIGN_PIXELS: f32 = 90.0;

/// One sign face being edited.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct SignEdit {
    position: [i32; 3],
    front: bool,
    lines: [String; SIGN_LINES],
    original: [String; SIGN_LINES],
    line: usize,
    /// Cursor position in characters within the current line.
    column: usize,
    base: NbtCompound,
    color: [u8; 4],
}

fn face_key(front: bool) -> &'static str {
    if front { "FrontText" } else { "BackText" }
}

fn split_lines(text: &str) -> [String; SIGN_LINES] {
    let mut lines: [String; SIGN_LINES] = Default::default();
    for (slot, line) in lines.iter_mut().zip(text.split('\n')) {
        *slot = line.to_owned();
    }
    lines
}

impl SignEdit {
    /// Starts editing `front` of the sign whose current compound is `base`.
    pub(crate) fn new(position: [i32; 3], front: bool, base: NbtCompound) -> Self {
        let face = base.compound(face_key(front));
        // Signs from before the two-sided format keep the front text at the root.
        let source = face.unwrap_or(&base);
        let text = source.string("Text").unwrap_or_default();
        let argb = source
            .integer("SignTextColor")
            .and_then(|value| i32::try_from(value).ok())
            .unwrap_or(-0x0100_0000);
        let [_, red, green, blue] = argb.to_be_bytes();
        let lines = split_lines(text);
        Self {
            position,
            front,
            original: lines.clone(),
            lines,
            line: 0,
            column: 0,
            base,
            color: [red, green, blue, 255],
        }
    }

    pub(crate) const fn position(&self) -> [i32; 3] {
        self.position
    }

    pub(crate) fn lines(&self) -> &[String; SIGN_LINES] {
        &self.lines
    }

    pub(crate) const fn cursor(&self) -> (usize, usize) {
        (self.line, self.column)
    }

    pub(crate) const fn color(&self) -> [u8; 4] {
        self.color
    }

    /// The edited text as the sign stores it, lines joined by newlines.
    pub(crate) fn text(&self) -> String {
        self.lines.join("\n")
    }

    pub(crate) fn changed(&self) -> bool {
        self.lines != self.original
    }

    fn byte_index(&self, column: usize) -> usize {
        self.lines[self.line]
            .char_indices()
            .nth(column)
            .map_or(self.lines[self.line].len(), |(index, _)| index)
    }

    fn line_chars(&self) -> usize {
        self.lines[self.line].chars().count()
    }

    /// Inserts `value` at the cursor when the widened line still satisfies `fits`.
    pub(crate) fn insert(&mut self, value: char, mut fits: impl FnMut(&str) -> bool) -> bool {
        if value.is_control() {
            return false;
        }
        let mut candidate = self.lines[self.line].clone();
        candidate.insert(self.byte_index(self.column), value);
        if !fits(&candidate) {
            return false;
        }
        self.lines[self.line] = candidate;
        self.column += 1;
        true
    }

    pub(crate) fn backspace(&mut self) {
        if self.column == 0 {
            return;
        }
        let end = self.byte_index(self.column);
        let start = self.byte_index(self.column - 1);
        self.lines[self.line].replace_range(start..end, "");
        self.column -= 1;
    }

    pub(crate) fn delete(&mut self) {
        if self.column >= self.line_chars() {
            return;
        }
        let start = self.byte_index(self.column);
        let end = self.byte_index(self.column + 1);
        self.lines[self.line].replace_range(start..end, "");
    }

    pub(crate) fn left(&mut self) {
        if self.column > 0 {
            self.column -= 1;
        } else if self.line > 0 {
            self.line -= 1;
            self.column = self.line_chars();
        }
    }

    pub(crate) fn right(&mut self) {
        if self.column < self.line_chars() {
            self.column += 1;
        } else if self.line + 1 < SIGN_LINES {
            self.line += 1;
            self.column = 0;
        }
    }

    pub(crate) fn up(&mut self) {
        if self.line > 0 {
            self.line -= 1;
            self.column = self.column.min(self.line_chars());
        }
    }

    pub(crate) fn down(&mut self) {
        if self.line + 1 < SIGN_LINES {
            self.line += 1;
            self.column = self.column.min(self.line_chars());
        }
    }

    pub(crate) fn home(&mut self) {
        self.column = 0;
    }

    pub(crate) fn end(&mut self) {
        self.column = self.line_chars();
    }

    /// Moves to the next line; `true` when already on the last, meaning the edit is finished.
    pub(crate) fn newline(&mut self) -> bool {
        if self.line + 1 >= SIGN_LINES {
            return true;
        }
        self.line += 1;
        self.column = 0;
        false
    }

    /// The whole sign compound with the edited face's text replaced, ready to send back. Both
    /// faces are always present, as servers require, and the block entity identity is kept.
    pub(crate) fn into_encoded_nbt(self) -> Vec<u8> {
        let text = self.text();
        let mut root = self.base;
        root.insert("id", NbtValue::String("Sign".into()));
        for (axis, value) in ["x", "y", "z"].into_iter().zip(self.position) {
            root.insert(axis, NbtValue::Int(value));
        }
        for front in [true, false] {
            let mut face = root.compound(face_key(front)).cloned().unwrap_or_default();
            if front == self.front {
                face.insert("Text", NbtValue::String(text.as_str().into()));
            } else if face.string("Text").is_none() {
                face.insert("Text", NbtValue::String("".into()));
            }
            root.insert(face_key(front), NbtValue::Compound(face));
        }
        root.encode_root()
    }
}

/// The open editor, if any.
#[derive(Clone, Debug, Default)]
pub(crate) struct SignEditor {
    active: Option<SignEdit>,
}

impl SignEditor {
    pub(crate) const fn is_open(&self) -> bool {
        self.active.is_some()
    }

    pub(crate) fn open(&mut self, edit: SignEdit) {
        self.active = Some(edit);
    }

    pub(crate) fn active(&self) -> Option<&SignEdit> {
        self.active.as_ref()
    }

    pub(crate) fn active_mut(&mut self) -> Option<&mut SignEdit> {
        self.active.as_mut()
    }

    /// Closes the editor and returns what was being edited.
    pub(crate) fn close(&mut self) -> Option<SignEdit> {
        self.active.take()
    }
}

#[cfg(test)]
mod tests;
