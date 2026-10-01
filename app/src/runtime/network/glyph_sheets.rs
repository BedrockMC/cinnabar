//! Session bitmap fonts: Unicode sheets plus the verified ASCII portion of `default8.png`.

use std::{io::Cursor, sync::Arc};

use assets::{CellGlyph, GlyphSheet, SHEET_GRID, extract_cells, texel_size_64};
use image::{ImageFormat, ImageReader, Limits};
use resource_pack::LayeredPackView;

use crate::ui_runtime::presentation::SessionGlyphSheets;

const MAX_SHEET_SOURCE_BYTES: u64 = 16 * 1024 * 1024;
const MAX_SHEET_SIDE: u32 = 4096;
const MAX_CELLS: usize = 32_768;

/// Crops every sheet present in the stack into glyph cells, or `None` when it has none. Each
/// sheet is dropped once cropped so a large one is not retained for the session.
pub(super) fn compile_session_glyphs(view: &LayeredPackView) -> Option<Arc<SessionGlyphSheets>> {
    let mut cells = read_named_sheet(view, 0, "font/default8.png")
        .map(|sheet| default_ascii_cells(&sheet))
        .unwrap_or_default();
    let has_default_ascii = !cells.is_empty();
    for high_byte in 0..=u8::MAX {
        let Some(sheet) = ["X", "x"]
            .iter()
            .find_map(|case| read_sheet(view, high_byte, case))
        else {
            continue;
        };
        for cell in extract_cells(&sheet) {
            if cells.len() < MAX_CELLS
                && !(has_default_ascii && (' '..='~').contains(&cell.codepoint))
            {
                cells.push(cell);
            }
        }
    }
    (!cells.is_empty()).then(|| Arc::new(SessionGlyphSheets::new(cells)))
}

/// Resolves either spelling of a Unicode sheet name through the shared bounded decoder.
fn read_sheet(view: &LayeredPackView, high_byte: u8, case: &str) -> Option<GlyphSheet> {
    let name = match case {
        "X" => format!("font/glyph_{high_byte:02X}.png"),
        _ => format!("font/glyph_{high_byte:02x}.png"),
    };
    read_named_sheet(view, high_byte, &name)
}

/// A malformed optional sheet falls through to the next valid layer.
fn read_named_sheet(view: &LayeredPackView, high_byte: u8, name: &str) -> Option<GlyphSheet> {
    view.stack().packs().iter().rev().find_map(|pack| {
        let bytes = pack
            .read_file_with_limit(name, MAX_SHEET_SOURCE_BYTES)
            .ok()
            .flatten()?;
        decode_sheet(high_byte, &bytes)
    })
}

/// Decodes only a valid, bounded 16-by-16 bitmap sheet.
fn decode_sheet(high_byte: u8, bytes: &[u8]) -> Option<GlyphSheet> {
    let (width, height) = ImageReader::with_format(Cursor::new(bytes), ImageFormat::Png)
        .into_dimensions()
        .ok()?;
    if width == 0
        || height == 0
        || width > MAX_SHEET_SIDE
        || height > MAX_SHEET_SIDE
        || !width.is_multiple_of(SHEET_GRID)
        || !height.is_multiple_of(SHEET_GRID)
    {
        return None;
    }
    let mut reader = ImageReader::with_format(Cursor::new(bytes), ImageFormat::Png);
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

/// `BitmapFont::loadFontData` scans the right edge; extended-byte remapping remains unverified.
fn default_ascii_cells(sheet: &GlyphSheet) -> Vec<CellGlyph> {
    let cell_width = sheet.width / SHEET_GRID;
    let cell_height = sheet.height / SHEET_GRID;
    let texel = texel_size_64(0, cell_width);
    let mut cells: Vec<_> = extract_cells(sheet)
        .into_iter()
        .filter(|cell| (' '..='~').contains(&cell.codepoint))
        .collect();
    for cell in &mut cells {
        if cell.codepoint == ' ' {
            cell.size = [0, 0];
            cell.rgba8 = Box::default();
            cell.draw_size_64 = [0, 0];
            // Vanilla's space is half the normalized eight-pixel cell.
            cell.advance_64 = (texel_size_64(0, 1) / 2) as i16;
            continue;
        }
        let index = u32::from(cell.codepoint);
        let origin = [
            index % SHEET_GRID * cell_width,
            index / SHEET_GRID * cell_height,
        ];
        let left = (0..cell_width)
            .find(|x| {
                (0..cell_height).any(|y| {
                    let pixel = ((origin[1] + y) * sheet.width + origin[0] + x) as usize;
                    sheet.rgba8[pixel * 4 + 3] != 0
                })
            })
            .unwrap_or(0);
        cell.bearing[0] = ((left * texel + 32) / 64) as i16;
        cell.advance_64 = if cell.size == [0, 0] {
            (texel_size_64(0, 1) / 8) as i16
        } else {
            (u32::try_from(cell.advance_64).unwrap_or(0) + left * texel).min(i16::MAX as u32) as i16
        };
    }
    cells
}

#[cfg(test)]
mod tests;
