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
const MAX_ARTWORKS: usize = 64;
/// Longest side kept for a list thumbnail (server logos, gamerpics, badges), so
/// a whole featured list fits the art pages beside banners.
pub(crate) const THUMBNAIL_SIDE: u32 = 128;
/// The start screen's title texture, which Cinnabar's own logo replaces.
pub(super) const TITLE_KEY: &str = "textures/ui/title";
/// Cinnabar's logo; the pack's title draws only if this fails to decode.
pub(crate) const BUILT_IN_TITLE: &[u8] = include_bytes!("../../../../assets/branding/title.png");

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
pub(super) fn load(
    paths: &[(String, u32)],
    oversized: &[(String, std::sync::Arc<[u8]>)],
    first_page: u16,
) -> MenuArtworkAtlas {
    let side = render::UI_ART_PAGE_SIDE;
    let mut unique = BTreeSet::new();
    // Big textures keep up to a whole page of detail; service art stays smaller.
    let whole_page = side - GUTTER * 2;
    let artwork = |path: &str, (pixels, width, height): (Vec<u8>, u32, u32)| Artwork {
        path: path.to_owned(),
        width,
        height,
        pixels,
    };
    let title = decode_bytes(BUILT_IN_TITLE, whole_page).map(|art| artwork(TITLE_KEY, art));
    let mut rest = paths
        .iter()
        .take(MAX_ARTWORKS)
        .filter(|(path, _)| !path.is_empty() && unique.insert(path.clone()))
        .filter_map(|(path, side)| Some(artwork(path, decode(Path::new(path), *side)?)))
        .collect::<Vec<_>>();
    rest.extend(
        oversized
            .iter()
            .filter(|(key, _)| key != TITLE_KEY && unique.insert(key.clone()))
            .filter_map(|(key, bytes)| Some(artwork(key, decode_bytes(bytes, whole_page)?))),
    );
    rest.sort_by(|a, b| b.height.cmp(&a.height).then(a.path.cmp(&b.path)));
    // The title packs first so later art can never crowd it out.
    let decoded: Vec<Artwork> = title.into_iter().chain(rest).collect();
    if decoded.is_empty() {
        return MenuArtworkAtlas::default();
    }
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
                glint: false,
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

fn decode(path: &Path, max_side: u32) -> Option<(Vec<u8>, u32, u32)> {
    let file = File::open(path).ok()?;
    let mut bytes = Vec::new();
    file.take((MAX_SOURCE_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .ok()?;
    if bytes.len() > MAX_SOURCE_BYTES {
        return None;
    }
    decode_bytes(&bytes, max_side.min(MAX_ARTWORK_SIDE))
}

/// Premultiplied RGBA8 of an image no larger than `max_side` on either axis.
fn decode_bytes(bytes: &[u8], max_side: u32) -> Option<(Vec<u8>, u32, u32)> {
    if bytes.is_empty() {
        return None;
    }
    let format = image::guess_format(bytes).ok()?;
    let dimensions = ImageReader::with_format(Cursor::new(bytes), format)
        .into_dimensions()
        .ok()?;
    if dimensions.0 == 0
        || dimensions.1 == 0
        || dimensions.0 > MAX_SOURCE_SIDE
        || dimensions.1 > MAX_SOURCE_SIDE
    {
        return None;
    }
    let mut reader = ImageReader::with_format(Cursor::new(bytes), format);
    let mut limits = Limits::default();
    limits.max_image_width = Some(MAX_SOURCE_SIDE);
    limits.max_image_height = Some(MAX_SOURCE_SIDE);
    limits.max_alloc = Some(MAX_DECODE_ALLOC);
    reader.limits(limits);
    let image = reader.decode().ok()?;
    let image = if image.width() > max_side || image.height() > max_side {
        image.resize(max_side, max_side, FilterType::Lanczos3)
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

/// Every downloaded artwork path the menu view can draw.
pub(super) fn view_paths(view: &crate::menu::MenuView) -> Vec<(String, u32)> {
    // The Servers tab shows the first experience until a server is picked.
    let shown = match view.feeds.selected_saved {
        Some(_) => None,
        None => Some(view.feeds.selected_featured.unwrap_or(0)),
    };
    let selected = shown
        .and_then(|index| {
            view.featured
                .iter()
                .chain(view.gatherings.iter())
                .nth(index)
        })
        .and_then(|server| view.feeds.details.get(&server.address));
    let thumbnails = view
        .featured
        .iter()
        .chain(view.gatherings.iter())
        .map(|server| server.image_path.clone())
        .chain(std::iter::once(view.feeds.profile.picture_path.clone()))
        .map(|path| (path, THUMBNAIL_SIDE));
    let full = home_art(&view.feeds.home)
        .into_iter()
        .chain(selected.into_iter().flat_map(|details| {
            details
                .screenshots
                .iter()
                .cloned()
                .chain(details.games.iter().map(|game| game.image_path.clone()))
        }))
        .chain(
            view.store
                .as_deref()
                .map(crate::store::StoreSnapshot::image_paths)
                .unwrap_or_default(),
        )
        .map(|path| (path, MAX_ARTWORK_SIDE));
    thumbnails
        .chain(full)
        .filter(|(path, _)| !path.is_empty())
        .collect()
}

/// The start screen's service art: messaging tile layers, the event badge and the persona head.
fn home_art(home: &crate::menu::MenuHome) -> Vec<String> {
    let mut paths = vec![home.persona_head.clone()];
    for art in [&home.play_art, &home.store_art].into_iter().flatten() {
        paths.extend([
            art.default_background.clone(),
            art.hover_background.clone(),
            art.default_foreground.clone(),
            art.hover_foreground.clone(),
        ]);
    }
    if let Some(event) = &home.live_event {
        paths.push(event.badge_path.clone());
    }
    paths
}
