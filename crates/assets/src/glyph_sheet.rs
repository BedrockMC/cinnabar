//! Bedrock `font/glyph_XX.png` sheets: a 16x16 grid of cells per high byte, packed as
//! column-trimmed glyphs into atlas pages. Private-use sheets (E0-F8) draw at one unscaled
//! px per texel; every other sheet is normalised so a cell is 8 px wide.

use crate::GlyphMetrics;

pub const SHEET_GRID: u32 = 16;
/// Width in unscaled px of a normalised (non-private-use) cell.
const NORMALISED_CELL_PX: u32 = 8;
/// Pixels from the line's baseline up to the top of a sheet cell.
const ASCENT_PX: i16 = 7;
const GUTTER: u32 = 1;
const PRIVATE_USE_SHEETS: std::ops::RangeInclusive<u8> = 0xe0..=0xf8;

/// One decoded sheet in straight-alpha RGBA8.
#[derive(Debug)]
pub struct GlyphSheet {
    pub high_byte: u8,
    pub width: u32,
    pub height: u32,
    pub rgba8: Box<[u8]>,
}

/// A packed glyph plus the size it is drawn at, in 1/64 unscaled px.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SheetGlyph {
    pub metrics: GlyphMetrics,
    pub draw_size_64: [u32; 2],
}

#[derive(Debug, Default)]
pub struct GlyphAtlas {
    pub pages: Vec<Box<[u8]>>,
    pub glyphs: Vec<SheetGlyph>,
}

/// Opaque column span `[left, right)` of a cell, or `None` when it is blank.
pub fn opaque_columns(sheet: &GlyphSheet, cell_x: u32, cell_y: u32) -> Option<[u32; 2]> {
    let (cell_w, cell_h) = (sheet.width / SHEET_GRID, sheet.height / SHEET_GRID);
    let opaque = |x: u32| {
        (0..cell_h).any(|y| {
            let index =
                (((cell_y * cell_h + y) * sheet.width + cell_x * cell_w + x) * 4 + 3) as usize;
            sheet.rgba8[index] != 0
        })
    };
    let left = (0..cell_w).find(|&x| opaque(x))?;
    let right = (left..cell_w).rev().find(|&x| opaque(x))? + 1;
    Some([left, right])
}

/// 1/64 unscaled px drawn per texel for a sheet whose cells are `cell_width` texels wide.
pub fn texel_size_64(high_byte: u8, cell_width: u32) -> u32 {
    if PRIVATE_USE_SHEETS.contains(&high_byte) {
        64
    } else {
        NORMALISED_CELL_PX * 64 / cell_width
    }
}

/// Advance in 1/64 px for a trimmed cell `columns` wide: its drawn width plus one pixel.
pub const fn advance_64(columns: u32, texel_64: u32) -> u32 {
    columns * texel_64 + 64
}

fn valid(sheet: &GlyphSheet) -> bool {
    let bytes = (sheet.width as usize)
        .checked_mul(sheet.height as usize)
        .and_then(|pixels| pixels.checked_mul(4));
    sheet.width != 0
        && sheet.height != 0
        && sheet.width.is_multiple_of(SHEET_GRID)
        && sheet.height.is_multiple_of(SHEET_GRID)
        && bytes == Some(sheet.rgba8.len())
}

/// Shelf-packs every cell of `sheets` into `side`-px square pages (at most `max_pages`),
/// private-use sheets first; glyph pages are numbered from `first_page`. Cells that do not
/// fit are dropped, invalid sheets skipped.
pub fn pack_glyph_sheets(
    sheets: &[GlyphSheet],
    first_page: u16,
    side: u32,
    max_pages: usize,
) -> GlyphAtlas {
    let mut ordered: Vec<&GlyphSheet> = sheets.iter().filter(|sheet| valid(sheet)).collect();
    ordered.sort_by_key(|sheet| {
        (
            !PRIVATE_USE_SHEETS.contains(&sheet.high_byte),
            sheet.high_byte,
        )
    });
    let mut atlas = GlyphAtlas::default();
    let mut cursor = [0u32; 2];
    let mut row_height = 0u32;
    let page_bytes = (side * side * 4) as usize;
    for sheet in ordered {
        let (cell_w, cell_h) = (sheet.width / SHEET_GRID, sheet.height / SHEET_GRID);
        let texel_64 = texel_size_64(sheet.high_byte, cell_w);
        let bearing_x = i16::from(PRIVATE_USE_SHEETS.contains(&sheet.high_byte));
        for index in 0..SHEET_GRID * SHEET_GRID {
            let (cell_x, cell_y) = (index % SHEET_GRID, index / SHEET_GRID);
            let Some(codepoint) = char::from_u32(u32::from(sheet.high_byte) << 8 | index) else {
                continue;
            };
            let Some([left, right]) = opaque_columns(sheet, cell_x, cell_y) else {
                atlas.glyphs.push(SheetGlyph {
                    metrics: GlyphMetrics {
                        codepoint,
                        page: first_page,
                        uv: [0; 4],
                        bearing: [0, 0],
                        advance_64: 0,
                    },
                    draw_size_64: [0, 0],
                });
                continue;
            };
            let width = right - left;
            let padded = [width + GUTTER * 2, cell_h + GUTTER * 2];
            if padded[0] > side || padded[1] > side {
                continue;
            }
            if cursor[0] + padded[0] > side {
                cursor = [0, cursor[1] + row_height];
                row_height = 0;
            }
            if atlas.pages.is_empty() || cursor[1] + padded[1] > side {
                if atlas.pages.len() >= max_pages {
                    return atlas;
                }
                if !atlas.pages.is_empty() {
                    cursor = [0, 0];
                    row_height = 0;
                }
                atlas.pages.push(vec![0; page_bytes].into());
            }
            let page = atlas.pages.last_mut().expect("page just ensured");
            for y in 0..padded[1] {
                let source_y = y.saturating_sub(GUTTER).min(cell_h - 1);
                for x in 0..padded[0] {
                    let source_x = left + x.saturating_sub(GUTTER).min(width - 1);
                    let source =
                        (((cell_y * cell_h + source_y) * sheet.width + cell_x * cell_w + source_x)
                            * 4) as usize;
                    let target = (((cursor[1] + y) * side + cursor[0] + x) * 4) as usize;
                    page[target..target + 4].copy_from_slice(&sheet.rgba8[source..source + 4]);
                }
            }
            let [uv_left, uv_top] = [cursor[0] + GUTTER, cursor[1] + GUTTER];
            atlas.glyphs.push(SheetGlyph {
                metrics: GlyphMetrics {
                    codepoint,
                    page: first_page + (atlas.pages.len() - 1) as u16,
                    uv: [
                        uv_left as u16,
                        uv_top as u16,
                        (uv_left + width) as u16,
                        (uv_top + cell_h) as u16,
                    ],
                    bearing: [bearing_x, -ASCENT_PX],
                    advance_64: advance_64(width, texel_64).min(i16::MAX as u32) as i16,
                },
                draw_size_64: [width * texel_64, cell_h * texel_64],
            });
            cursor[0] += padded[0];
            row_height = row_height.max(padded[1]);
        }
    }
    atlas
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 16-cell-wide sheet of `cell`-px cells with the given `(index, columns)` filled solid.
    fn sheet(high_byte: u8, cell: u32, filled: &[(u32, [u32; 2])]) -> GlyphSheet {
        let side = cell * SHEET_GRID;
        let mut rgba8 = vec![0u8; (side * side * 4) as usize];
        for &(index, [left, right]) in filled {
            let (cell_x, cell_y) = (index % SHEET_GRID, index / SHEET_GRID);
            for y in 0..cell {
                for x in left..right {
                    let at = (((cell_y * cell + y) * side + cell_x * cell + x) * 4) as usize;
                    rgba8[at..at + 4].copy_from_slice(&[255, 255, 255, 255]);
                }
            }
        }
        GlyphSheet {
            high_byte,
            width: side,
            height: side,
            rgba8: rgba8.into(),
        }
    }

    #[test]
    fn opaque_columns_follow_the_cell_content() {
        let sheet = sheet(0xe0, 16, &[(0, [2, 10]), (17, [0, 16])]);
        assert_eq!(opaque_columns(&sheet, 0, 0), Some([2, 10]));
        assert_eq!(opaque_columns(&sheet, 1, 1), Some([0, 16]));
        assert_eq!(opaque_columns(&sheet, 5, 5), None);
    }

    #[test]
    fn private_use_sheets_draw_one_px_per_texel() {
        let atlas = pack_glyph_sheets(&[sheet(0xe1, 8, &[(3, [0, 8]), (0xff, [2, 4])])], 7, 256, 4);
        let by_char = |c: char| {
            atlas
                .glyphs
                .iter()
                .find(|g| g.metrics.codepoint == c)
                .unwrap()
        };
        let full = by_char('\u{e103}');
        assert_eq!(full.metrics.page, 7);
        assert_eq!(full.draw_size_64, [8 * 64, 8 * 64]);
        assert_eq!(full.metrics.advance_64, 9 * 64);
        assert_eq!(full.metrics.bearing, [1, -7]);
        let narrow = by_char('\u{e1ff}');
        assert_eq!(narrow.draw_size_64[0], 2 * 64);
        assert_eq!(narrow.metrics.advance_64, 3 * 64);
        assert_eq!(by_char('\u{e100}').metrics.advance_64, 0);
        assert_eq!(atlas.glyphs.len(), 256);
        assert_eq!(atlas.pages.len(), 1);
    }

    #[test]
    fn other_sheets_normalise_cells_to_eight_px() {
        let atlas = pack_glyph_sheets(&[sheet(0x4e, 16, &[(1, [0, 16])])], 0, 256, 1);
        let glyph = atlas
            .glyphs
            .iter()
            .find(|g| g.metrics.codepoint == '\u{4e01}')
            .unwrap();
        assert_eq!(glyph.metrics.uv[2] - glyph.metrics.uv[0], 16);
        assert_eq!(glyph.draw_size_64, [8 * 64, 8 * 64]);
        assert_eq!(glyph.metrics.advance_64, 9 * 64);
    }

    #[test]
    fn private_use_sheets_pack_first_and_overflow_is_dropped() {
        let sheets = [
            sheet(0x00, 16, &[(1, [0, 16])]),
            sheet(0xe0, 16, &[(1, [0, 16])]),
        ];
        let atlas = pack_glyph_sheets(&sheets, 0, 256, 1);
        assert_eq!(atlas.glyphs[0].metrics.codepoint, '\u{e000}');
        let full: Vec<_> = (0..256).map(|i| (i, [0, 16])).collect();
        let many: Vec<_> = (0xe0..0xf0).map(|b| sheet(b, 16, &full)).collect();
        let capped = pack_glyph_sheets(&many, 0, 256, 2);
        assert_eq!(capped.pages.len(), 2);
        assert!(capped.glyphs.len() < many.len() * 256);
    }

    #[test]
    fn malformed_sheets_are_skipped() {
        let bad = GlyphSheet {
            high_byte: 0xe0,
            width: 17,
            height: 16,
            rgba8: vec![0; 17 * 16 * 4].into(),
        };
        let short = GlyphSheet {
            high_byte: 0xe0,
            width: 16,
            height: 16,
            rgba8: vec![0; 4].into(),
        };
        assert!(
            pack_glyph_sheets(&[bad, short], 0, 256, 4)
                .glyphs
                .is_empty()
        );
    }
}
