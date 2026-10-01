//! A joined server's resource-pack UI textures: `textures/**/*.png` with their
//! `*.json` sidecars shadow the vanilla carrier's textures of the same path.
//! Each decodes and shelf-packs into the reserved 256x256 dynamic pages only
//! when a rendered screen draws it; one larger than a page packs downscaled.
//! Undecodable images are skipped.

use std::{
    cell::RefCell,
    collections::{BTreeMap, BTreeSet},
    io::Cursor,
    path::PathBuf,
};

use image::{ImageReader, Limits};
use json_ui::{TextureMeta, parse_texture_meta};
use render::UiTexturePage;

use super::remote_images::{RemoteImages, RemoteState, is_remote};

/// Image extensions a texture path may resolve to, in lookup order.
const IMAGE_EXTENSIONS: [&str; 4] = [".png", ".tga", ".jpg", ".jpeg"];
/// Where vanilla's in-package resource pack sits; its files read from the local
/// vanilla pack.
pub(super) const VANILLA_IN_PACKAGE: &str = "resource_packs/vanilla/";

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
        (IMAGE_EXTENSIONS
            .iter()
            .any(|extension| path.ends_with(extension))
            || path.ends_with(".json"))
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

/// Where a resident server texture sits: its page (relative to the first
/// server page) and pixel rect.
#[derive(Clone, Copy, Debug)]
pub(super) struct ServerTexture {
    pub(super) page: u16,
    pub(super) rect: [u16; 4],
}

/// A pack texture known by its header, decoded only when a screen draws it.
/// One larger than a page packs downscaled to fit; UVs stay normalized.
#[derive(Clone)]
struct Source {
    bytes: std::sync::Arc<[u8]>,
    size: [u32; 2],
    packed: [u32; 2],
    meta: Option<TextureMeta>,
}

/// One reserved page: its pixels, shelf cursor, residents, and last use.
struct Page {
    pixels: Vec<u8>,
    cursor: [u32; 3],
    keys: Vec<String>,
    used: u64,
    image: UiTexturePage,
}

impl Page {
    fn blank() -> Option<Self> {
        let pixels = vec![0; (PAGE_SIDE * PAGE_SIDE * 4) as usize];
        let image = UiTexturePage::owned([PAGE_SIDE; 2], pixels.clone().into()).ok()?;
        Some(Self {
            pixels,
            cursor: [0; 3],
            keys: Vec::new(),
            used: 0,
            image,
        })
    }

    /// A shelf slot for `size`, advancing the cursor, or `None` when full.
    fn allocate(&mut self, size: [u32; 2]) -> Option<[u32; 2]> {
        let [mut x, mut y, mut shelf] = self.cursor;
        if x + size[0] > PAGE_SIDE {
            (x, y, shelf) = (0, y + shelf + GUTTER, 0);
        }
        if y + size[1] > PAGE_SIDE {
            return None;
        }
        self.cursor = [x + size[0] + GUTTER, y, shelf.max(size[1])];
        Some([x, y])
    }
}

/// The server pack's UI textures, packed on demand into a fixed number of
/// reserved pages: a texture becomes resident when a rendered screen draws it,
/// and a full atlas evicts its least recently drawn page.
#[derive(Default)]
pub(super) struct ServerAtlas {
    sources: BTreeMap<String, Source>,
    /// Source keys by lowercase spelling.
    folded: BTreeMap<String, String>,
    /// Vanilla images and downloaded URLs, found on first use; `None` when absent.
    extra: RefCell<BTreeMap<String, Option<Source>>>,
    /// The local vanilla resource pack vanilla image paths read from.
    vanilla: Option<PathBuf>,
    remote: Option<RemoteImages>,
    resident: BTreeMap<String, ServerTexture>,
    pages: Vec<Page>,
    images: Vec<UiTexturePage>,
    max_pages: usize,
    clock: u64,
    dirty: bool,
    /// Drawn textures too big for a page, for the full-resolution art pages.
    oversized: BTreeMap<String, std::sync::Arc<[u8]>>,
}

impl ServerAtlas {
    /// Index the pack's pngs (by path stem) and their sidecars; nothing decodes.
    pub(super) fn new(files: &[(String, Vec<u8>)], max_pages: usize) -> Self {
        let sidecars: BTreeMap<&str, TextureMeta> = files
            .iter()
            .filter_map(|(path, bytes)| {
                let stem = path.strip_suffix(".json")?;
                stem.starts_with("textures/").then_some(())?;
                let value = serde_json::from_slice(bytes).ok()?;
                Some((stem, parse_texture_meta(&value)?))
            })
            .collect();
        // A path names its image without an extension; `.png` wins over `.tga`
        // and `.jpg`, so it inserts last.
        let ranked = IMAGE_EXTENSIONS.iter().rev().flat_map(|extension| {
            files
                .iter()
                .filter_map(move |(path, bytes)| Some((path.strip_suffix(extension)?, bytes)))
        });
        let sources = ranked
            .filter_map(|(stem, bytes)| {
                stem.starts_with("textures/").then_some(())?;
                let found = source(bytes.as_slice().into(), sidecars.get(stem).copied())?;
                Some((stem.to_owned(), found))
            })
            .collect::<BTreeMap<_, _>>();
        let folded = sources
            .keys()
            .map(|key| (key.to_ascii_lowercase(), key.clone()))
            .collect();
        Self {
            sources,
            folded,
            max_pages,
            dirty: true,
            ..Self::default()
        }
    }

    /// Also read vanilla images from `vanilla` and download remote ones.
    pub(super) fn with_fallbacks(
        mut self,
        vanilla: Option<PathBuf>,
        remote: Option<RemoteImages>,
    ) -> Self {
        self.vanilla = vanilla;
        self.remote = remote;
        self
    }

    /// Layout metadata for a pack texture, resident or not.
    pub(super) fn meta(&self, key: &str) -> Option<TextureMeta> {
        self.sources.get(key).map(Source::meta)
    }

    /// Layout metadata for a vanilla image or a downloaded URL, reading or
    /// requesting it on first use.
    pub(super) fn fallback_meta(&self, key: &str) -> Option<TextureMeta> {
        self.fallback(key).as_ref().map(Source::meta)
    }

    /// The vanilla image or downloaded URL behind `key`. A vanilla miss is
    /// remembered; a URL still loading is asked again next time.
    fn fallback(&self, key: &str) -> Option<Source> {
        if let Some(found) = self.extra.borrow().get(key) {
            return found.clone();
        }
        let (found, settled) = if is_remote(key) {
            match self.remote.as_ref()?.state(key) {
                RemoteState::Ready(bytes) => (source(bytes, None), true),
                RemoteState::Failed => (None, true),
                RemoteState::Loading => (None, false),
            }
        } else {
            let root = self.vanilla.as_ref()?;
            let relative = key.strip_prefix(VANILLA_IN_PACKAGE).unwrap_or(key);
            let found = (key.starts_with("textures/") || relative != key).then(|| {
                IMAGE_EXTENSIONS.iter().find_map(|extension| {
                    let bytes = std::fs::read(root.join(format!("{relative}{extension}"))).ok()?;
                    source(bytes.into(), None)
                })
            });
            (found.flatten(), true)
        };
        if settled {
            self.extra
                .borrow_mut()
                .insert(key.to_owned(), found.clone());
        }
        found
    }

    /// The source key spelled `folded` in lowercase.
    pub(super) fn folded(&self, folded: &str) -> Option<&str> {
        self.folded.get(folded).map(String::as_str)
    }

    pub(super) fn placement(&self, key: &str) -> Option<ServerTexture> {
        self.resident.get(key).copied()
    }

    /// Drawn textures a page had to shrink, by key, with their source bytes.
    pub(super) fn oversized(&self) -> Vec<(String, std::sync::Arc<[u8]>)> {
        self.oversized
            .iter()
            .map(|(key, bytes)| (key.clone(), std::sync::Arc::clone(bytes)))
            .collect()
    }

    /// Page images in order, for the reserved dynamic pages.
    pub(super) fn images(&self) -> &[UiTexturePage] {
        &self.images
    }

    /// `true` once since the page images last changed.
    pub(super) fn take_dirty(&mut self) -> bool {
        std::mem::take(&mut self.dirty)
    }

    /// Make every texture in `keys` resident for this frame: pack textures, then
    /// vanilla images and downloaded URLs. A texture
    /// that fits nowhere without evicting one drawn this frame is left out.
    pub(super) fn require<'a>(&mut self, keys: impl IntoIterator<Item = &'a str>) {
        self.clock += 1;
        // Mark what is already resident first, so a miss never evicts a page
        // this frame still draws.
        let mut missing = Vec::new();
        for key in keys {
            match self.resident.get(key) {
                Some(texture) => self.pages[usize::from(texture.page)].used = self.clock,
                None => missing.push(key),
            }
        }
        let mut changed = Vec::new();
        for key in missing {
            if !self.resident.contains_key(key)
                && let Some(page) = self.place(key)
            {
                changed.push(page);
            }
        }
        changed.sort_unstable();
        changed.dedup();
        for page in changed {
            let page = &mut self.pages[page];
            if let Ok(image) = UiTexturePage::owned([PAGE_SIDE; 2], page.pixels.clone().into()) {
                page.image = image;
            }
        }
        if self.images.len() != self.pages.len()
            || self
                .images
                .iter()
                .zip(&self.pages)
                .any(|(image, page)| image.identity() != page.image.identity())
        {
            self.images = self.pages.iter().map(|page| page.image.clone()).collect();
            self.dirty = true;
        }
    }

    /// Decode and pack `key`, returning the page it landed on.
    fn place(&mut self, key: &str) -> Option<usize> {
        let source = match self.sources.get(key) {
            Some(source) => source.clone(),
            None => self.fallback(key)?,
        };
        if source.packed != source.size && self.oversized.len() < MAX_OVERSIZED {
            self.oversized
                .insert(key.to_owned(), std::sync::Arc::clone(&source.bytes));
        }
        let size = source.packed;
        let (index, origin) = self.slot(size)?;
        let rgba = decode(&source.bytes, size)?;
        let page = &mut self.pages[index];
        let row_bytes = PAGE_SIDE as usize * 4;
        let width = size[0] as usize * 4;
        for row in 0..size[1] as usize {
            let start = (origin[1] as usize + row) * row_bytes + origin[0] as usize * 4;
            page.pixels[start..start + width]
                .copy_from_slice(&rgba[row * width..(row + 1) * width]);
        }
        page.keys.push(key.to_owned());
        page.used = self.clock;
        let rect = [origin[0], origin[1], size[0], size[1]].map(|value| value as u16);
        self.resident.insert(
            key.to_owned(),
            ServerTexture {
                page: index as u16,
                rect,
            },
        );
        Some(index)
    }

    /// Room for `size`: an existing page, a new page, or the least recently
    /// drawn page not drawn this frame, cleared.
    fn slot(&mut self, size: [u32; 2]) -> Option<(usize, [u32; 2])> {
        for (index, page) in self.pages.iter_mut().enumerate() {
            if let Some(origin) = page.allocate(size) {
                return Some((index, origin));
            }
        }
        if self.pages.len() < self.max_pages {
            self.pages.push(Page::blank()?);
            let index = self.pages.len() - 1;
            return Some((index, self.pages[index].allocate(size)?));
        }
        let clock = self.clock;
        let (index, _) = self
            .pages
            .iter()
            .enumerate()
            .filter(|(_, page)| page.used < clock)
            .min_by_key(|(_, page)| page.used)?;
        let page = &mut self.pages[index];
        for key in page.keys.drain(..) {
            self.resident.remove(&key);
        }
        page.pixels.fill(0);
        page.cursor = [0; 3];
        Some((index, page.allocate(size)?))
    }
}

impl Source {
    fn meta(&self) -> TextureMeta {
        self.meta.unwrap_or(TextureMeta {
            base_size: self.size.map(f64::from),
            nineslice: None,
        })
    }
}

/// A decodable image as a source, with an optional sidecar.
fn source(bytes: std::sync::Arc<[u8]>, meta: Option<TextureMeta>) -> Option<Source> {
    let size = dimensions(&bytes)?;
    Some(Source {
        bytes,
        size,
        packed: fitted(size),
        meta,
    })
}

/// Oversized textures remembered for the art pages.
const MAX_OVERSIZED: usize = 16;

/// Largest source side decoded; bigger images are skipped.
const MAX_SOURCE_SIDE: u32 = 4096;

/// A png's size from its header, when within the decode bound.
fn dimensions(bytes: &[u8]) -> Option<[u32; 2]> {
    let (width, height) = reader(bytes)?.into_dimensions().ok()?;
    (width > 0 && height > 0 && width <= MAX_SOURCE_SIDE && height <= MAX_SOURCE_SIDE)
        .then_some([width, height])
}

/// A reader for a pack image by its content, as the vanilla loader detects it:
/// a `.png` path may hold TGA data, which carries no magic number.
fn reader(bytes: &[u8]) -> Option<ImageReader<Cursor<&[u8]>>> {
    let mut reader = ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .ok()?;
    if reader.format().is_none() {
        reader.set_format(image::ImageFormat::Tga);
    }
    Some(reader)
}

/// `size` scaled down, keeping its aspect, to fit one page.
fn fitted(size: [u32; 2]) -> [u32; 2] {
    let largest = size[0].max(size[1]);
    if largest <= PAGE_SIDE {
        return size;
    }
    size.map(|side| (u64::from(side) * u64::from(PAGE_SIDE) / u64::from(largest)).max(1) as u32)
}

/// RGBA8 pixels of a bounded image at `size`, or `None` when undecodable.
fn decode(bytes: &[u8], size: [u32; 2]) -> Option<Vec<u8>> {
    let mut reader = reader(bytes)?;
    let mut limits = Limits::default();
    limits.max_image_width = Some(MAX_SOURCE_SIDE);
    limits.max_image_height = Some(MAX_SOURCE_SIDE);
    reader.limits(limits);
    let image = reader.decode().ok()?.into_rgba8();
    let image = if [image.width(), image.height()] == size {
        image
    } else {
        image::imageops::resize(
            &image,
            size[0],
            size[1],
            image::imageops::FilterType::Triangle,
        )
    };
    Some(image.into_raw())
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

    fn tga(width: u32, height: u32) -> Vec<u8> {
        let mut bytes = Vec::new();
        image::RgbaImage::from_pixel(width, height, image::Rgba([4, 5, 6, 255]))
            .write_to(&mut Cursor::new(&mut bytes), image::ImageFormat::Tga)
            .unwrap();
        bytes
    }

    // Only drawn textures pack; sidecars feed layout; TGA data in a png path reads.
    #[test]
    fn textures_pack_when_drawn_with_their_sidecars() {
        let files = vec![
            ("textures/ui/button.png".to_owned(), png(16, 8)),
            (
                "textures/ui/button.json".to_owned(),
                br#"{ "nineslice_size": 2, "base_size": [16, 8] }"#.to_vec(),
            ),
            ("textures/ui/other.png".to_owned(), tga(4, 4)),
            ("textures/ui/wide.png".to_owned(), png(512, 4)),
        ];
        let mut atlas = ServerAtlas::new(&files, 1);
        assert!(
            atlas
                .meta("textures/ui/button")
                .unwrap()
                .nineslice
                .is_some()
        );
        assert_eq!(
            atlas.meta("textures/ui/wide").unwrap().base_size,
            [512.0, 4.0]
        );
        assert_eq!(
            atlas.folded("textures/ui/button"),
            Some("textures/ui/button")
        );
        atlas.require(["textures/ui/button"]);
        assert_eq!(
            atlas.placement("textures/ui/button").unwrap().rect,
            [0, 0, 16, 8]
        );
        assert!(atlas.placement("textures/ui/other").is_none());
        assert_eq!(
            atlas.meta("textures/ui/other").unwrap().base_size,
            [4.0, 4.0]
        );
        assert_eq!(atlas.images().len(), 1);
        assert!(atlas.take_dirty());
        atlas.require(["textures/ui/button"]);
        assert!(!atlas.take_dirty(), "a resident texture changes no page");
        atlas.require(["textures/ui/wide"]);
        assert_eq!(
            atlas.placement("textures/ui/wide").unwrap().rect[2..],
            [256, 2],
            "an oversized texture packs downscaled"
        );
        let oversized = atlas.oversized();
        assert_eq!(oversized.len(), 1, "and is offered to the art pages");
        assert_eq!(oversized[0].0, "textures/ui/wide");
    }

    // A full atlas evicts the page drawn least recently, never one drawn this frame.
    #[test]
    fn a_full_atlas_evicts_the_least_recently_drawn_page() {
        let files: Vec<_> = (0..3)
            .map(|index| (format!("textures/ui/t{index}.png"), png(200, 200)))
            .collect();
        let mut atlas = ServerAtlas::new(&files, 2);
        atlas.require(["textures/ui/t0"]);
        atlas.require(["textures/ui/t1"]);
        atlas.require(["textures/ui/t1", "textures/ui/t2"]);
        assert!(atlas.placement("textures/ui/t0").is_none());
        assert!(atlas.placement("textures/ui/t1").is_some());
        assert!(atlas.placement("textures/ui/t2").is_some());
        atlas.require(["textures/ui/t0", "textures/ui/t1", "textures/ui/t2"]);
        let resident = (0..3)
            .filter(|index| atlas.placement(&format!("textures/ui/t{index}")).is_some())
            .count();
        assert_eq!(
            resident, 2,
            "two pages hold two of three; the rest is left out"
        );
    }
}
