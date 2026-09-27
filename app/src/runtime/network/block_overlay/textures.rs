//! Terrain texture keys, flipbooks, and bounded image decoding from the stack.

use std::{collections::HashMap, io::Cursor};

use image::{ImageFormat, ImageReader, Limits};
use resource_pack::{LayeredPackView, normalize_jsonc};
use serde_json::Value;

const MAX_TEXTURE_SOURCE_BYTES: usize = 4 * 1024 * 1024;
const MAX_TEXTURE_SIDE: u32 = 1024;
const MAX_DECODE_ALLOC: u64 = 16 * 1024 * 1024;
const MAX_CATALOG_ENTRIES: usize = 16_384;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct DecodedTexture {
    pub(super) width: u32,
    pub(super) height: u32,
    pub(super) rgba8: Box<[u8]>,
}

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
        let mut terrain = HashMap::new();
        for layer in view.read_layers("textures/terrain_texture.json") {
            let Some(Value::Object(data)) =
                parse_json(&layer).map(|mut root| root["texture_data"].take())
            else {
                continue;
            };
            for (key, entry) in data {
                if terrain.len() >= MAX_CATALOG_ENTRIES && !terrain.contains_key(&key) {
                    break;
                }
                if let Some(path) = first_texture_path(&entry["textures"]) {
                    terrain.insert(key, path);
                }
            }
        }
        let mut flipbooks = HashMap::new();
        for layer in view.read_layers("textures/flipbook_textures.json") {
            let Some(Value::Array(entries)) = parse_json(&layer) else {
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

    /// Decodes the image a terrain key names, trying `.png` then `.tga`.
    pub(super) fn decode(&self, key: &str) -> Option<DecodedTexture> {
        let path = self.terrain.get(key)?;
        [("png", ImageFormat::Png), ("tga", ImageFormat::Tga)]
            .into_iter()
            .find_map(|(extension, format)| {
                let bytes = self.view.read(&format!("{path}.{extension}"))?;
                decode_image(&bytes, format)
            })
    }
}

fn parse_json(bytes: &[u8]) -> Option<Value> {
    serde_json::from_slice(&normalize_jsonc(bytes)?).ok()
}

/// A texture entry is a path, an object with `path`, or a variation list whose
/// first element is used.
fn first_texture_path(value: &Value) -> Option<String> {
    let path = match value {
        Value::String(path) => path.as_str(),
        Value::Object(entry) => entry.get("path")?.as_str()?,
        Value::Array(entries) => return first_texture_path(entries.first()?),
        _ => return None,
    };
    let path = path.trim().trim_start_matches("./");
    (!path.is_empty()).then(|| path.to_owned())
}

fn decode_image(bytes: &[u8], format: ImageFormat) -> Option<DecodedTexture> {
    if bytes.is_empty() || bytes.len() > MAX_TEXTURE_SOURCE_BYTES {
        return None;
    }
    let (width, height) = ImageReader::with_format(Cursor::new(bytes), format)
        .into_dimensions()
        .ok()?;
    if width == 0 || height == 0 || width > MAX_TEXTURE_SIDE || height > MAX_TEXTURE_SIDE {
        return None;
    }
    let mut reader = ImageReader::with_format(Cursor::new(bytes), format);
    let mut limits = Limits::default();
    limits.max_image_width = Some(MAX_TEXTURE_SIDE);
    limits.max_image_height = Some(MAX_TEXTURE_SIDE);
    limits.max_alloc = Some(MAX_DECODE_ALLOC);
    reader.limits(limits);
    let rgba8 = reader
        .decode()
        .ok()?
        .into_rgba8()
        .into_raw()
        .into_boxed_slice();
    Some(DecodedTexture {
        width,
        height,
        rgba8,
    })
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
