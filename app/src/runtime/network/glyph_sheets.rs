//! Session glyph sheets from the stack's `font/glyph_XX.png`; the highest pack's sheet wins.

use std::{io::Cursor, sync::Arc};

use assets::{GlyphSheet, extract_cells};
use image::{ImageFormat, ImageReader, Limits};
use resource_pack::LayeredPackView;

use crate::ui_runtime::presentation::SessionGlyphSheets;

const MAX_SHEET_SOURCE_BYTES: u64 = 16 * 1024 * 1024;
const MAX_SHEET_SIDE: u32 = 4096;
const MAX_CELLS: usize = 32_768;

/// Crops every sheet present in the stack into glyph cells, or `None` when it has none. Each
/// sheet is dropped once cropped so a large one is not retained for the session.
pub(super) fn compile_session_glyphs(view: &LayeredPackView) -> Option<Arc<SessionGlyphSheets>> {
    let mut cells = Vec::new();
    for high_byte in 0..=u8::MAX {
        let Some(sheet) = ["X", "x"]
            .iter()
            .find_map(|case| read_sheet(view, high_byte, case))
        else {
            continue;
        };
        for cell in extract_cells(&sheet) {
            if cells.len() < MAX_CELLS {
                cells.push(cell);
            }
        }
    }
    (!cells.is_empty()).then(|| Arc::new(SessionGlyphSheets { cells }))
}

fn read_sheet(view: &LayeredPackView, high_byte: u8, case: &str) -> Option<GlyphSheet> {
    let name = match case {
        "X" => format!("font/glyph_{high_byte:02X}.png"),
        _ => format!("font/glyph_{high_byte:02x}.png"),
    };
    let bytes = view.read_capped(&name, MAX_SHEET_SOURCE_BYTES)?;
    let (width, height) = ImageReader::with_format(Cursor::new(&bytes), ImageFormat::Png)
        .into_dimensions()
        .ok()?;
    if width == 0 || height == 0 || width > MAX_SHEET_SIDE || height > MAX_SHEET_SIDE {
        return None;
    }
    let mut reader = ImageReader::with_format(Cursor::new(&bytes), ImageFormat::Png);
    let mut limits = Limits::default();
    limits.max_image_width = Some(MAX_SHEET_SIDE);
    limits.max_image_height = Some(MAX_SHEET_SIDE);
    limits.max_alloc = Some(u64::from(MAX_SHEET_SIDE) * u64::from(MAX_SHEET_SIDE) * 8);
    reader.limits(limits);
    let rgba8 = reader
        .decode()
        .ok()?
        .into_rgba8()
        .into_raw()
        .into_boxed_slice();
    Some(GlyphSheet {
        high_byte,
        width,
        height,
        rgba8,
    })
}

#[cfg(test)]
mod tests;
