//! A joined server's resource-pack UI textures: `textures/**/*.png` decode and
//! shelf-pack into reserved 256x256 dynamic pages, with their `*.json`
//! sidecars, and shadow the vanilla carrier's textures of the same path.
//! Oversized or undecodable images are skipped, as the carrier compiler skips
//! them, and so is whatever overflows the reserved pages.

use std::{
    collections::{BTreeMap, BTreeSet},
    io::Cursor,
};

use image::{ImageReader, Limits};
use json_ui::{TextureMeta, parse_texture_meta};
use render::UiTexturePage;

/// Side of a dynamic UI page, which also bounds one server texture.
const PAGE_SIDE: u32 = 256;
const GUTTER: u32 = 1;

/// A session's server resource-pack UI: each pack's `ui/**/*.json`, lowest
/// precedence first (each layer merges over the ones below), and the winning
/// `textures/**` images and sidecars the pack ui references.
#[derive(Debug, Default)]
pub(crate) struct ServerUiPack {
    pub(crate) ui_layers: Vec<Vec<(String, Vec<u8>)>>,
    pub(crate) textures: Vec<(String, Vec<u8>)>,
}

impl ServerUiPack {
    pub(crate) fn is_empty(&self) -> bool {
        self.ui_layers.iter().all(Vec::is_empty)
    }

    /// Whether a pack texture file is worth packing: a png or sidecar in a
    /// directory the ui references (always `textures/ui/`).
    pub(crate) fn wants_texture(dirs: &BTreeSet<String>, path: &str) -> bool {
        (path.ends_with(".png") || path.ends_with(".json"))
            && (path.starts_with("textures/ui/") || dirs.iter().any(|dir| path.starts_with(dir)))
    }

    /// Directories of every `textures/...` string literal in the ui json.
    pub(crate) fn referenced_texture_dirs(layers: &[Vec<(String, Vec<u8>)>]) -> BTreeSet<String> {
        let mut dirs = BTreeSet::new();
        for (_, bytes) in layers.iter().flatten() {
            let text = String::from_utf8_lossy(bytes);
            for (at, _) in text.match_indices("\"textures/") {
                let literal = &text[at + 1..];
                let end = literal.find('"').unwrap_or(literal.len());
                if let Some((dir, _)) = literal[..end].rsplit_once('/') {
                    dirs.insert(format!("{dir}/"));
                }
            }
        }
        dirs
    }
}

/// One packed server texture: its page (relative to the first server page), its
/// pixel rect, and its sidecar metadata.
#[derive(Clone, Copy, Debug)]
pub(super) struct ServerTexture {
    pub(super) page: u16,
    pub(super) rect: [u16; 4],
    pub(super) meta: Option<TextureMeta>,
}

pub(super) struct PackedServerTextures {
    pub(super) pages: Vec<UiTexturePage>,
    pub(super) textures: BTreeMap<String, ServerTexture>,
}

/// Decode and pack the pack's UI textures into at most `max_pages` pages.
pub(super) fn pack(files: &[(String, Vec<u8>)], max_pages: usize) -> PackedServerTextures {
    let side = [PAGE_SIDE; 2];
    let sidecars: BTreeMap<&str, TextureMeta> = files
        .iter()
        .filter_map(|(path, bytes)| {
            let stem = path.strip_suffix(".json")?;
            stem.starts_with("textures/").then_some(())?;
            let value = serde_json::from_slice(bytes).ok()?;
            Some((stem, parse_texture_meta(&value)?))
        })
        .collect();
    let mut decoded: Vec<(String, u32, u32, Vec<u8>)> = files
        .iter()
        .filter_map(|(path, bytes)| {
            let stem = path.strip_suffix(".png")?;
            stem.starts_with("textures/").then_some(())?;
            let (width, height, rgba) = decode(bytes)?;
            Some((stem.to_owned(), width, height, rgba))
        })
        .collect();
    decoded.sort_by(|a, b| (b.2, &a.0).cmp(&(a.2, &b.0)));

    let row_bytes = side[0] as usize * 4;
    let mut buffers: Vec<Vec<u8>> = Vec::new();
    let mut textures = BTreeMap::new();
    let (mut x, mut y, mut shelf) = (0u32, 0u32, 0u32);
    for (path, width, height, rgba) in decoded {
        if width > side[0] || height > side[1] {
            continue;
        }
        if x + width > side[0] {
            x = 0;
            y += shelf + GUTTER;
            shelf = 0;
        }
        if buffers.is_empty() || y + height > side[1] {
            if buffers.len() == max_pages {
                break;
            }
            buffers.push(vec![0; row_bytes * side[1] as usize]);
            (x, y, shelf) = (0, 0, 0);
        }
        let page = buffers.last_mut().expect("a page was just ensured");
        for row in 0..height as usize {
            let source = &rgba[row * width as usize * 4..(row + 1) * width as usize * 4];
            let start = (y as usize + row) * row_bytes + x as usize * 4;
            page[start..start + source.len()].copy_from_slice(source);
        }
        let meta = sidecars.get(path.as_str()).copied();
        textures.insert(
            path,
            ServerTexture {
                page: (buffers.len() - 1) as u16,
                rect: [x as u16, y as u16, width as u16, height as u16],
                meta,
            },
        );
        x += width + GUTTER;
        shelf = shelf.max(height);
    }
    let pages = buffers
        .into_iter()
        .filter_map(|pixels| UiTexturePage::owned(side, pixels.into()).ok())
        .collect();
    PackedServerTextures { pages, textures }
}

/// RGBA8 pixels of a bounded image, or `None` when oversized or undecodable.
fn decode(bytes: &[u8]) -> Option<(u32, u32, Vec<u8>)> {
    let probe = ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .ok()?;
    let (width, height) = probe.into_dimensions().ok()?;
    if width == 0 || height == 0 || width > PAGE_SIDE || height > PAGE_SIDE {
        return None;
    }
    let mut reader = ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .ok()?;
    let mut limits = Limits::default();
    limits.max_image_width = Some(PAGE_SIDE);
    limits.max_image_height = Some(PAGE_SIDE);
    reader.limits(limits);
    let image = reader.decode().ok()?.into_rgba8();
    Some((width, height, image.into_raw()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn png(width: u32, height: u32) -> Vec<u8> {
        let mut bytes = Vec::new();
        image::RgbaImage::from_pixel(width, height, image::Rgba([1, 2, 3, 255]))
            .write_to(&mut Cursor::new(&mut bytes), image::ImageFormat::Png)
            .unwrap();
        bytes
    }

    #[test]
    fn packs_textures_with_their_sidecars_and_skips_oversized_ones() {
        let files = vec![
            ("textures/ui/button.png".to_owned(), png(16, 8)),
            (
                "textures/ui/button.json".to_owned(),
                br#"{ "nineslice_size": 2, "base_size": [16, 8] }"#.to_vec(),
            ),
            ("textures/ui/huge.png".to_owned(), png(300, 4)),
            ("ui/screen.json".to_owned(), b"{}".to_vec()),
        ];
        let packed = pack(&files, 1);
        assert_eq!(packed.pages.len(), 1);
        let button = packed.textures["textures/ui/button"];
        assert_eq!(button.rect, [0, 0, 16, 8]);
        assert!(button.meta.and_then(|meta| meta.nineslice).is_some());
        assert!(!packed.textures.contains_key("textures/ui/huge"));
    }
}
