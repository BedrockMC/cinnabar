//! Bounded decoding and atlas packing for service-provided launcher artwork.
//!
//! The authenticated Go catalog downloads remote images into the local cache.
//! This module treats those files as untrusted input: reads, decoded dimensions,
//! allocation and output pages are all capped before artwork enters the
//! retained UI texture array's full-resolution art pages.

use std::{
    collections::{BTreeSet, HashMap},
    fs::File,
    io::{Cursor, Read},
    path::Path,
};

use image::{ImageReader, Limits, imageops::FilterType};

use super::IconRef;

const MAX_SOURCE_BYTES: usize = 8 * 1024 * 1024;
const MAX_SOURCE_SIDE: u32 = 4_096;
const MAX_DECODE_ALLOC: u64 = 64 * 1024 * 1024;
/// Largest side artwork keeps; bigger sources scale down, smaller stay native.
const MAX_ARTWORK_SIDE: u32 = 512;
const GUTTER: u32 = 1;
const MAX_ARTWORKS: usize = 32;

#[derive(Default)]
pub(super) struct MenuArtworkAtlas {
    pub(super) pages: Vec<render::UiTexturePage>,
    pub(super) refs: HashMap<String, IconRef>,
}

/// Decoded artwork: premultiplied RGBA8 and its size.
struct Artwork {
    path: String,
    width: u32,
    height: u32,
    pixels: Vec<u8>,
}

/// Shelf-packs the artwork at `paths` into the full-resolution art pages that
/// start at texture page `first_page`; what does not fit is left out.
pub(super) fn load(paths: &[String], first_page: u16) -> MenuArtworkAtlas {
    let side = render::UI_ART_PAGE_SIDE;
    let mut unique = BTreeSet::new();
    let mut decoded = paths
        .iter()
        .take(MAX_ARTWORKS)
        .filter(|path| !path.is_empty() && unique.insert((*path).clone()))
        .filter_map(|path| {
            let (pixels, width, height) = decode(Path::new(path))?;
            Some(Artwork {
                path: path.clone(),
                width,
                height,
                pixels,
            })
        })
        .collect::<Vec<_>>();
    if decoded.is_empty() {
        return MenuArtworkAtlas::default();
    }
    decoded.sort_by(|a, b| b.height.cmp(&a.height).then(a.path.cmp(&b.path)));
    let page_bytes = side as usize * side as usize * 4;
    let mut buffers: Vec<Vec<u8>> = Vec::new();
    let mut refs = HashMap::with_capacity(decoded.len());
    let (mut page, mut x, mut y, mut shelf) = (0usize, GUTTER, GUTTER, 0u32);
    for art in decoded {
        if x + art.width + GUTTER > side {
            x = GUTTER;
            y += shelf + GUTTER;
            shelf = 0;
        }
        if y + art.height + GUTTER > side {
            page += 1;
            x = GUTTER;
            y = GUTTER;
            shelf = 0;
        }
        if page >= render::MAX_UI_ART_PAGES {
            break;
        }
        while buffers.len() <= page {
            buffers.push(vec![0; page_bytes]);
        }
        let row_bytes = art.width as usize * 4;
        for row in 0..art.height as usize {
            let target = ((y as usize + row) * side as usize + x as usize) * 4;
            buffers[page][target..target + row_bytes]
                .copy_from_slice(&art.pixels[row * row_bytes..(row + 1) * row_bytes]);
        }
        let Ok(texture_page) = u16::try_from(usize::from(first_page) + page) else {
            break;
        };
        let (left, top) = (x as u16, y as u16);
        refs.insert(
            art.path,
            IconRef {
                page: texture_page,
                uv: [left, top, left + art.width as u16, top + art.height as u16],
            },
        );
        x += art.width + GUTTER;
        shelf = shelf.max(art.height);
    }
    let pages = buffers
        .into_iter()
        .map(|pixels| {
            render::UiTexturePage::owned([side, side], std::sync::Arc::from(pixels))
                .expect("art pages have exact checked dimensions")
        })
        .collect();
    MenuArtworkAtlas { pages, refs }
}

fn decode(path: &Path) -> Option<(Vec<u8>, u32, u32)> {
    let file = File::open(path).ok()?;
    let mut bytes = Vec::new();
    file.take((MAX_SOURCE_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .ok()?;
    if bytes.is_empty() || bytes.len() > MAX_SOURCE_BYTES {
        return None;
    }
    let format = image::guess_format(&bytes).ok()?;
    let dimensions = ImageReader::with_format(Cursor::new(&bytes), format)
        .into_dimensions()
        .ok()?;
    if dimensions.0 == 0
        || dimensions.1 == 0
        || dimensions.0 > MAX_SOURCE_SIDE
        || dimensions.1 > MAX_SOURCE_SIDE
    {
        return None;
    }
    let mut reader = ImageReader::with_format(Cursor::new(&bytes), format);
    let mut limits = Limits::default();
    limits.max_image_width = Some(MAX_SOURCE_SIDE);
    limits.max_image_height = Some(MAX_SOURCE_SIDE);
    limits.max_alloc = Some(MAX_DECODE_ALLOC);
    reader.limits(limits);
    let image = reader.decode().ok()?;
    let image = if image.width() > MAX_ARTWORK_SIDE || image.height() > MAX_ARTWORK_SIDE {
        image.resize(MAX_ARTWORK_SIDE, MAX_ARTWORK_SIDE, FilterType::Lanczos3)
    } else {
        image
    };
    let image = image.into_rgba8();
    let (width, height) = image.dimensions();
    let mut pixels = image.into_raw();
    for pixel in pixels.chunks_exact_mut(4) {
        let alpha = u16::from(pixel[3]);
        pixel[0] = ((u16::from(pixel[0]) * alpha + 127) / 255) as u8;
        pixel[1] = ((u16::from(pixel[1]) * alpha + 127) / 255) as u8;
        pixel[2] = ((u16::from(pixel[2]) * alpha + 127) / 255) as u8;
    }
    Some((pixels, width, height))
}
