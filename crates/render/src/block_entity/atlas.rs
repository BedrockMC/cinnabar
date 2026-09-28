//! Block-entity texture atlas: the carrier's packed static pixels plus a fixed strip
//! below them for runtime-rasterized sign text canvases.

use std::{collections::BTreeMap, sync::Arc};

use assets::RuntimeBlockEntityAssets;

/// Size of one text canvas cell in atlas pixels.
pub const TEXT_CELL: [u32; 2] = [96, 48];
const TEXT_COLUMNS: u32 = 10;
const TEXT_ROWS: u32 = 10;
/// Height of the dynamic strip appended below the static atlas.
pub const DYNAMIC_STRIP_HEIGHT: u32 = TEXT_CELL[1] * TEXT_ROWS;
pub const TEXT_SLOT_COUNT: usize = (TEXT_COLUMNS * TEXT_ROWS) as usize;

/// A pixel rect in the full atlas.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct AtlasRect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

/// A pack texture placed in the atlas, with the size its model UVs were authored against.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TextureRef {
    pub rect: AtlasRect,
    pub logical: [f32; 2],
}

impl TextureRef {
    /// Converts a `[u, v, width, height]` texel rect of the logical texture to atlas-pixel
    /// `[u0, v0, u1, v1]`, so a higher-resolution replacement texture still maps.
    #[must_use]
    pub fn rect_uv(&self, texels: [f32; 4]) -> [f32; 4] {
        let scale_x = self.rect.width / self.logical[0];
        let scale_y = self.rect.height / self.logical[1];
        [
            self.rect.x + texels[0] * scale_x,
            self.rect.y + texels[1] * scale_y,
            self.rect.x + (texels[0] + texels[2]) * scale_x,
            self.rect.y + (texels[1] + texels[3]) * scale_y,
        ]
    }
}

#[derive(Debug)]
pub struct BlockEntityAtlas {
    size: [u32; 2],
    static_height: u32,
    static_rgba8: Arc<[u8]>,
    placements: BTreeMap<Box<str>, AtlasRect>,
    identity: [u8; 32],
}

impl BlockEntityAtlas {
    #[must_use]
    pub fn from_assets(assets: &RuntimeBlockEntityAssets) -> Self {
        let [width, static_height] = assets.atlas_size();
        Self {
            size: [width, static_height + DYNAMIC_STRIP_HEIGHT],
            static_height,
            static_rgba8: Arc::clone(assets.atlas_rgba8()),
            placements: assets
                .placements()
                .iter()
                .map(|placement| {
                    (
                        placement.name.clone(),
                        AtlasRect {
                            x: placement.x as f32,
                            y: placement.y as f32,
                            width: placement.width as f32,
                            height: placement.height as f32,
                        },
                    )
                })
                .collect(),
            identity: assets.identity(),
        }
    }

    /// Full atlas size including the dynamic strip.
    #[must_use]
    pub const fn size(&self) -> [u32; 2] {
        self.size
    }

    #[must_use]
    pub const fn static_height(&self) -> u32 {
        self.static_height
    }

    #[must_use]
    pub fn static_rgba8(&self) -> &Arc<[u8]> {
        &self.static_rgba8
    }

    #[must_use]
    pub const fn identity(&self) -> [u8; 32] {
        self.identity
    }

    /// A packed pack texture by pack-relative path without extension.
    #[must_use]
    pub fn texture(&self, name: &str, logical: [f32; 2]) -> Option<TextureRef> {
        self.placements.get(name).map(|rect| TextureRef {
            rect: *rect,
            logical,
        })
    }

    /// One RGBA8 texel of a packed texture, in the texture's own pixel coordinates.
    #[must_use]
    pub fn texel(&self, name: &str, x: u32, y: u32) -> Option<[u8; 4]> {
        let rect = self.placements.get(name)?;
        if x as f32 >= rect.width || y as f32 >= rect.height {
            return None;
        }
        let row = rect.y as u32 + y;
        let column = rect.x as u32 + x;
        let start = (row as usize * self.size[0] as usize + column as usize) * 4;
        self.static_rgba8.get(start..start + 4)?.try_into().ok()
    }

    /// The rect of dynamic text cell `slot`.
    #[must_use]
    pub fn text_cell(&self, slot: usize) -> Option<AtlasRect> {
        let slot = u32::try_from(slot)
            .ok()
            .filter(|slot| *slot < TEXT_COLUMNS * TEXT_ROWS)?;
        Some(AtlasRect {
            x: ((slot % TEXT_COLUMNS) * TEXT_CELL[0]) as f32,
            y: (self.static_height + (slot / TEXT_COLUMNS) * TEXT_CELL[1]) as f32,
            width: TEXT_CELL[0] as f32,
            height: TEXT_CELL[1] as f32,
        })
    }
}

/// Least-recently-used allocation of text canvases into the dynamic strip.
#[derive(Debug)]
pub struct DynamicText {
    width: usize,
    pixels: Vec<u8>,
    keys: Vec<Option<u64>>,
    last_used: Vec<u64>,
    clock: u64,
    revision: u64,
}

impl DynamicText {
    #[must_use]
    pub fn new(atlas_width: u32) -> Self {
        let width = atlas_width as usize;
        Self {
            width,
            pixels: vec![0; width * DYNAMIC_STRIP_HEIGHT as usize * 4],
            keys: vec![None; TEXT_SLOT_COUNT],
            last_used: vec![0; TEXT_SLOT_COUNT],
            clock: 0,
            revision: 0,
        }
    }

    #[must_use]
    pub const fn revision(&self) -> u64 {
        self.revision
    }

    #[must_use]
    pub fn pixels(&self) -> &[u8] {
        &self.pixels
    }

    /// The slot holding `key`, rasterizing `make` (a `TEXT_CELL` RGBA8 canvas) into the
    /// least recently used slot on a miss. `None` when the canvas has the wrong size.
    pub fn slot(&mut self, key: u64, make: impl FnOnce() -> Vec<u8>) -> Option<usize> {
        self.clock += 1;
        if let Some(slot) = self.keys.iter().position(|entry| *entry == Some(key)) {
            self.last_used[slot] = self.clock;
            return Some(slot);
        }
        let canvas = make();
        let [cell_width, cell_height] = TEXT_CELL.map(|value| value as usize);
        if canvas.len() != cell_width * cell_height * 4 {
            return None;
        }
        let slot = self
            .keys
            .iter()
            .position(Option::is_none)
            .or_else(|| (0..TEXT_SLOT_COUNT).min_by_key(|slot| self.last_used[*slot]))?;
        let column = slot % TEXT_COLUMNS as usize;
        let row = slot / TEXT_COLUMNS as usize;
        for line in 0..cell_height {
            let target = ((row * cell_height + line) * self.width + column * cell_width) * 4;
            let source = line * cell_width * 4;
            self.pixels[target..target + cell_width * 4]
                .copy_from_slice(&canvas[source..source + cell_width * 4]);
        }
        self.keys[slot] = Some(key);
        self.last_used[slot] = self.clock;
        self.revision = self.revision.wrapping_add(1);
        Some(slot)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn canvas(value: u8) -> Vec<u8> {
        vec![value; (TEXT_CELL[0] * TEXT_CELL[1] * 4) as usize]
    }

    #[test]
    fn rect_uv_scales_with_texture_resolution() {
        let texture = TextureRef {
            rect: AtlasRect {
                x: 100.0,
                y: 200.0,
                width: 128.0,
                height: 128.0,
            },
            logical: [64.0, 64.0],
        };
        assert_eq!(
            texture.rect_uv([8.0, 4.0, 8.0, 8.0]),
            [116.0, 208.0, 132.0, 224.0]
        );
    }

    #[test]
    fn text_slots_reuse_hits_and_evict_least_recently_used() {
        let mut text = DynamicText::new(1024);
        let first = text.slot(1, || canvas(1)).unwrap();
        assert_eq!(
            text.slot(1, || panic!("a hit must not rasterize")),
            Some(first)
        );
        assert_eq!(text.revision(), 1);
        for key in 2..=TEXT_SLOT_COUNT as u64 {
            text.slot(key, || canvas(key as u8)).unwrap();
        }
        // Key 1 was touched first and is now the oldest; a new key takes its slot.
        let replaced = text.slot(1_000, || canvas(9)).unwrap();
        assert_eq!(replaced, first);
        assert_eq!(text.pixels()[0], 9);
        assert!(text.slot(2_000, || vec![0; 3]).is_none());
    }
}
