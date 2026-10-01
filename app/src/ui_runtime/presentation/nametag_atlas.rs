//! Name-tag lines rasterized at font resolution into the shared atlas the tag billboards sample.

use std::{collections::HashMap, sync::Arc};

use assets::RuntimeFontCatalog;
use render::NAMETAG_ATLAS_SIDE;
use ui::{
    FONT_DESIGN_PIXEL_TEXELS, TEXT_BASELINE_64, TEXT_LINE_HEIGHT_64, TextLayoutCache,
    TextLayoutRequest, TextStyle, UiScale,
};

/// Widest line laid out before it would wrap, in font texels.
const MAX_LINE_TEXELS: u32 = NAMETAG_ATLAS_SIDE;

/// RGBA8 texels of one UI texture page a glyph samples.
#[derive(Clone, Copy)]
pub(crate) struct GlyphPage<'a> {
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) rgba8: &'a [u8],
}

/// The font's own pages, which lead the UI texture pages.
pub(crate) fn font_page(font: &RuntimeFontCatalog, page: usize) -> Option<GlyphPage<'_>> {
    font.pages().get(page).map(|page| GlyphPage {
        width: page.width,
        height: page.height,
        rgba8: &page.rgba8,
    })
}

/// One rasterized line: its atlas cell in texels and its width in font pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct AtlasLine {
    pub(super) cell: [u32; 4],
    pub(super) width_px: f32,
    /// Font pixels from the line box's top to the cell's top (negative for tall glyphs).
    pub(super) top_px: f32,
}

/// Shelf-packed line cells, rebuilt from scratch when a frame's lines no longer fit.
#[derive(Default)]
pub(crate) struct NametagAtlas {
    pixels: Vec<u8>,
    lines: HashMap<Arc<str>, AtlasLine>,
    shelf: [u32; 3],
    published: Option<Arc<[u8]>>,
    revision: u64,
}

impl NametagAtlas {
    /// The cell of `text`, rasterizing it on first use; `None` when the font cannot lay it out.
    pub(super) fn line<'p>(
        &mut self,
        text: &Arc<str>,
        font: &RuntimeFontCatalog,
        layouts: &mut TextLayoutCache,
        pages: &impl Fn(usize) -> Option<GlyphPage<'p>>,
    ) -> Option<AtlasLine> {
        if let Some(line) = self.lines.get(text) {
            return Some(*line);
        }
        let (width, height, top, rgba8, advance) = rasterize(text, font, layouts, pages)?;
        let origin = self.allocate(width, height)?;
        let side = NAMETAG_ATLAS_SIDE as usize;
        for row in 0..height as usize {
            let source = row * width as usize * 4;
            let target = ((origin[1] as usize + row) * side + origin[0] as usize) * 4;
            self.pixels[target..target + width as usize * 4]
                .copy_from_slice(&rgba8[source..source + width as usize * 4]);
        }
        let line = AtlasLine {
            cell: [origin[0], origin[1], width, height],
            width_px: advance as f32 / FONT_DESIGN_PIXEL_TEXELS as f32,
            top_px: top as f32 / FONT_DESIGN_PIXEL_TEXELS as f32,
        };
        self.lines.insert(Arc::clone(text), line);
        self.published = None;
        Some(line)
    }

    /// Forgets every line so the next frame packs only what it draws.
    pub(super) fn reset(&mut self) {
        self.lines.clear();
        self.shelf = [0; 3];
        self.pixels.fill(0);
        self.published = None;
    }

    pub(super) fn has_room_for(&self, texts: usize) -> bool {
        self.lines.len() + texts < MAX_ATLAS_LINES
    }

    /// The texels to upload and their revision, bumped whenever a line was added.
    pub(super) fn publish(&mut self) -> (Arc<[u8]>, u64) {
        if self.published.is_none() {
            self.revision += 1;
            let pixels = if self.pixels.is_empty() {
                vec![0; (NAMETAG_ATLAS_SIDE * NAMETAG_ATLAS_SIDE * 4) as usize]
            } else {
                self.pixels.clone()
            };
            self.published = Some(pixels.into());
        }
        (
            Arc::clone(self.published.as_ref().expect("just published")),
            self.revision,
        )
    }

    fn allocate(&mut self, width: u32, height: u32) -> Option<[u32; 2]> {
        let side = NAMETAG_ATLAS_SIDE;
        if width > side || height > side {
            return None;
        }
        if self.pixels.is_empty() {
            self.pixels = vec![0; (side * side * 4) as usize];
        }
        let [mut x, mut y, mut shelf_height] = self.shelf;
        if x + width > side {
            (x, y, shelf_height) = (0, y + shelf_height, 0);
        }
        if y + height > side {
            return None;
        }
        self.shelf = [x + width, y, shelf_height.max(height)];
        Some([x, y])
    }
}

/// Lines kept before the atlas is rebuilt, far above any frame's visible tags.
const MAX_ATLAS_LINES: usize = 4096;

/// `text` as one unwrapped line of font texels: `(width, height, RGBA8)`. Glyph texels keep
/// their own colour (image glyphs) times the `§` colour, white when unset.
fn rasterize<'p>(
    text: &str,
    font: &RuntimeFontCatalog,
    layouts: &mut TextLayoutCache,
    pages: &impl Fn(usize) -> Option<GlyphPage<'p>>,
) -> Option<(u32, u32, i32, Vec<u8>, u32)> {
    let layout = layouts
        .layout(TextLayoutRequest {
            text,
            style: TextStyle::default(),
            width_64: MAX_LINE_TEXELS * 64,
            line_height_64: TEXT_LINE_HEIGHT_64,
            baseline_64: TEXT_BASELINE_64,
            scale: UiScale::default(),
            font,
            wrap: Default::default(),
        })
        .ok()?;
    let advance = layout.size_64()[0].div_ceil(64).max(1);
    let glyphs = || layout.glyphs().iter().filter(|glyph| glyph.line == 0);
    let top = glyphs()
        .map(|glyph| glyph.bounds_64[1].div_euclid(64))
        .min()
        .unwrap_or(0)
        .min(0);
    let bottom = glyphs()
        .map(|glyph| (glyph.bounds_64[3] + 63).div_euclid(64))
        .max()
        .unwrap_or(0)
        .max(TEXT_LINE_HEIGHT_64.div_ceil(64) as i32);
    let right = glyphs()
        .map(|glyph| (glyph.bounds_64[2] + 63).div_euclid(64))
        .max()
        .unwrap_or(0)
        .max(advance as i32);
    let (width, height) = (right as u32, (bottom - top) as u32);
    let mut canvas = vec![0u8; (width * height * 4) as usize];
    for glyph in glyphs() {
        let Some(page) = pages(usize::from(glyph.page)) else {
            continue;
        };
        let tint = glyph.style.color.rgb().unwrap_or([255; 3]);
        // Sheet glyphs are drawn scaled into their bounds, so sample the source nearest-texel.
        let mut bounds = glyph.bounds_64.map(|value| value as f32 / 64.0);
        bounds[1] -= top as f32;
        bounds[3] -= top as f32;
        let [u0, v0, u1, v1] = glyph.uv.map(f32::from);
        let (source_width, source_height) = (u1 - u0 + 1.0, v1 - v0 + 1.0);
        let (dest_width, dest_height) = (bounds[2] - bounds[0], bounds[3] - bounds[1]);
        if dest_width <= 0.0 || dest_height <= 0.0 {
            continue;
        }
        for dy in bounds[1].floor().max(0.0) as u32..(bounds[3].ceil() as u32).min(height) {
            for dx in bounds[0].floor().max(0.0) as u32..(bounds[2].ceil() as u32).min(width) {
                let fx = (dx as f32 + 0.5 - bounds[0]) / dest_width;
                let fy = (dy as f32 + 0.5 - bounds[1]) / dest_height;
                if !(0.0..1.0).contains(&fx) || !(0.0..1.0).contains(&fy) {
                    continue;
                }
                let sx = (u0 + fx * source_width) as u32;
                let sy = (v0 + fy * source_height) as u32;
                if sx >= page.width || sy >= page.height {
                    continue;
                }
                let source = ((sy * page.width + sx) * 4) as usize;
                let Some(texel) = page.rgba8.get(source..source + 4) else {
                    continue;
                };
                if texel[3] == 0 {
                    continue;
                }
                let target = ((dy * width + dx) * 4) as usize;
                for channel in 0..3 {
                    canvas[target + channel] =
                        (u16::from(texel[channel]) * u16::from(tint[channel]) / 255) as u8;
                }
                canvas[target + 3] = texel[3];
            }
        }
    }
    Some((width, height, top, canvas, advance))
}
