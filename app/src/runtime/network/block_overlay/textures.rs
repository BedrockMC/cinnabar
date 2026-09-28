//! Terrain texture keys and flipbooks resolved across the stack.

use std::collections::HashMap;

use resource_pack::LayeredPackView;
use serde_json::Value;

pub(super) use super::super::resource_packs::DecodedTexture;
use super::super::resource_packs::{
    MAX_CATALOG_ENTRIES, decode_pack_texture, parse_pack_json, texture_key_paths,
};

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(super) struct Flipbook {
    pub(super) frames: Option<Vec<u32>>,
    pub(super) ticks_per_frame: u32,
    pub(super) blend: bool,
}

/// Texture keys resolved across the stack; a higher pack replaces a key.
pub(super) struct TextureCatalog<'a> {
    view: &'a LayeredPackView,
    terrain: HashMap<String, String>,
    flipbooks: HashMap<String, Flipbook>,
}

impl<'a> TextureCatalog<'a> {
    pub(super) fn new(view: &'a LayeredPackView) -> Self {
        let terrain = texture_key_paths(view, "textures/terrain_texture.json");
        let mut flipbooks = HashMap::new();
        for layer in view.read_layers("textures/flipbook_textures.json") {
            let Some(Value::Array(entries)) = parse_pack_json(&layer) else {
                continue;
            };
            for entry in entries.iter().take(MAX_CATALOG_ENTRIES) {
                let Some(tile) = entry["atlas_tile"].as_str() else {
                    continue;
                };
                let frames = entry["frames"].as_array().map(|frames| {
                    frames
                        .iter()
                        .filter_map(|frame| {
                            frame.as_u64().and_then(|frame| u32::try_from(frame).ok())
                        })
                        .collect()
                });
                let ticks = entry["ticks_per_frame"]
                    .as_u64()
                    .unwrap_or(1)
                    .clamp(1, 1 << 16);
                flipbooks.insert(
                    tile.to_owned(),
                    Flipbook {
                        frames,
                        ticks_per_frame: ticks as u32,
                        blend: entry["blend_frames"].as_bool().unwrap_or(true),
                    },
                );
            }
        }
        Self {
            view,
            terrain,
            flipbooks,
        }
    }

    pub(super) fn flipbook(&self, key: &str) -> Option<&Flipbook> {
        self.flipbooks.get(key)
    }

    /// Decodes the image a terrain key names.
    pub(super) fn decode(&self, key: &str) -> Option<DecodedTexture> {
        decode_pack_texture(self.view, self.terrain.get(key)?)
    }
}

/// Splits a vertical strip into square frames when it is one; otherwise the
/// whole image is the single frame.
pub(super) fn flipbook_frames(
    texture: &DecodedTexture,
    flipbook: &Flipbook,
) -> Vec<DecodedTexture> {
    let side = texture.width;
    if texture.height <= side || !texture.height.is_multiple_of(side) {
        return vec![texture.clone()];
    }
    let count = texture.height / side;
    let order = flipbook
        .frames
        .clone()
        .filter(|frames| !frames.is_empty())
        .unwrap_or_else(|| (0..count).collect());
    let frame_bytes = (side * side * 4) as usize;
    order
        .into_iter()
        .take(256)
        .map(|frame| {
            let start = (frame.min(count - 1) as usize) * frame_bytes;
            DecodedTexture {
                width: side,
                height: side,
                rgba8: texture.rgba8[start..start + frame_bytes].into(),
            }
        })
        .collect()
}

/// Resamples to a square power-of-two tile: exact halvings in linear light when
/// possible, nearest-neighbour otherwise so pixel art stays crisp.
pub(super) fn resample_square(texture: &DecodedTexture, tile: u32) -> Box<[u8]> {
    if texture.width == texture.height && texture.width.is_power_of_two() && texture.width >= tile {
        let mut pixels = texture.rgba8.clone();
        let mut size = texture.width;
        while size > tile {
            pixels = assets::downsample_linear_premultiplied(&pixels, size);
            size /= 2;
        }
        return pixels;
    }
    let mut pixels = Vec::with_capacity((tile * tile * 4) as usize);
    for y in 0..tile {
        let source_y = (u64::from(y) * u64::from(texture.height) / u64::from(tile)) as usize;
        for x in 0..tile {
            let source_x = (u64::from(x) * u64::from(texture.width) / u64::from(tile)) as usize;
            let offset = (source_y * texture.width as usize + source_x) * 4;
            pixels.extend_from_slice(&texture.rgba8[offset..offset + 4]);
        }
    }
    pixels.into_boxed_slice()
}
