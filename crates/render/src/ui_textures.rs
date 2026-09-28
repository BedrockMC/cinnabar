//! Immutable logical UI pages and checked dimension-bucket admission.

use std::sync::Arc;

use assets::RuntimeFontCatalog;
use sha2::{Digest, Sha256};

use crate::ui::{
    MAX_UI_TEXTURE_BYTES, MAX_UI_TEXTURE_LAYERS, MAX_UI_TEXTURE_SIDE, UiRenderRejectReason,
};

pub const MAX_UI_TEXTURE_BUCKETS: usize = 8;
/// Replaceable 256x256 pages after the static UI pages.
pub const MAX_UI_DYNAMIC_PAGES: usize = 10;

#[derive(Clone, Debug, Eq, PartialEq)]
enum Pixels {
    Owned(Arc<[u8]>),
    Font {
        catalog: Arc<RuntimeFontCatalog>,
        page: usize,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UiTexturePage {
    dimensions: [u32; 2],
    identity: [u8; 32],
    pixels: Pixels,
}

impl UiTexturePage {
    pub fn owned(dimensions: [u32; 2], pixels: Arc<[u8]>) -> Result<Self, UiRenderRejectReason> {
        let expected = page_bytes(dimensions)?;
        if pixels.len() != expected {
            return Err(UiRenderRejectReason::TextureByteLengthInvalid {
                actual: pixels.len(),
                expected,
            });
        }
        Ok(Self {
            dimensions,
            identity: Sha256::digest(&pixels).into(),
            pixels: Pixels::Owned(pixels),
        })
    }

    pub fn font(
        catalog: Arc<RuntimeFontCatalog>,
        page: usize,
    ) -> Result<Self, UiRenderRejectReason> {
        let source = catalog
            .pages()
            .get(page)
            .ok_or(UiRenderRejectReason::InvalidTextureExtent)?;
        let dimensions = [source.width, source.height];
        if source.rgba8.len() != page_bytes(dimensions)? {
            return Err(UiRenderRejectReason::InvalidTextureExtent);
        }
        // RuntimeFontCatalog can only be obtained through its authenticated decoder.
        Ok(Self {
            dimensions,
            identity: source.pixels_sha256,
            pixels: Pixels::Font { catalog, page },
        })
    }

    pub fn pixels(&self) -> &[u8] {
        match &self.pixels {
            Pixels::Owned(pixels) => pixels,
            Pixels::Font { catalog, page } => &catalog.pages()[*page].rgba8,
        }
    }

    pub const fn dimensions(&self) -> [u32; 2] {
        self.dimensions
    }
    pub const fn identity(&self) -> [u8; 32] {
        self.identity
    }
}

fn page_bytes([width, height]: [u32; 2]) -> Result<usize, UiRenderRejectReason> {
    if width == 0 || height == 0 || width > MAX_UI_TEXTURE_SIDE || height > MAX_UI_TEXTURE_SIDE {
        return Err(UiRenderRejectReason::InvalidTextureExtent);
    }
    (width as usize)
        .checked_mul(height as usize)
        .and_then(|n| n.checked_mul(4))
        .ok_or(UiRenderRejectReason::InvalidTextureExtent)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct UiTextureLocation {
    pub bucket: usize,
    pub layer: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct UiTextureBucket {
    pub dimensions: [u32; 2],
    pub layers: u32,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UiTexturePlan {
    buckets: Box<[UiTextureBucket]>,
    locations: Box<[UiTextureLocation]>,
    bytes: usize,
}

impl UiTexturePlan {
    /// Dry plan all pages, including blank reservations, before allocating pixels.
    pub fn new(dimensions: &[[u32; 2]]) -> Result<Self, UiRenderRejectReason> {
        if dimensions.is_empty() || dimensions.len() > MAX_UI_TEXTURE_LAYERS as usize {
            return Err(UiRenderRejectReason::InvalidTextureExtent);
        }
        let mut buckets = Vec::<UiTextureBucket>::new();
        let mut locations = Vec::with_capacity(dimensions.len());
        let mut bytes = 0usize;
        for &dimensions in dimensions {
            bytes = bytes
                .checked_add(page_bytes(dimensions)?)
                .ok_or(UiRenderRejectReason::InvalidTextureExtent)?;
            if bytes > MAX_UI_TEXTURE_BYTES {
                return Err(UiRenderRejectReason::TextureByteLimitExceeded {
                    actual: bytes,
                    limit: MAX_UI_TEXTURE_BYTES,
                });
            }
            let bucket =
                if let Some(index) = buckets.iter().position(|b| b.dimensions == dimensions) {
                    index
                } else {
                    if buckets.len() == MAX_UI_TEXTURE_BUCKETS {
                        return Err(UiRenderRejectReason::InvalidTextureExtent);
                    }
                    buckets.push(UiTextureBucket {
                        dimensions,
                        layers: 0,
                    });
                    buckets.len() - 1
                };
            locations.push(UiTextureLocation {
                bucket,
                layer: buckets[bucket].layers,
            });
            buckets[bucket].layers += 1;
        }
        Ok(Self {
            buckets: buckets.into(),
            locations: locations.into(),
            bytes,
        })
    }

    pub fn validate_device(
        &self,
        max_side: u32,
        max_layers: u32,
    ) -> Result<(), UiRenderRejectReason> {
        if self
            .buckets
            .iter()
            .any(|b| b.dimensions.iter().any(|&side| side > max_side) || b.layers > max_layers)
        {
            return Err(UiRenderRejectReason::InvalidTextureExtent);
        }
        Ok(())
    }
    pub fn buckets(&self) -> &[UiTextureBucket] {
        &self.buckets
    }
    pub fn locations(&self) -> &[UiTextureLocation] {
        &self.locations
    }
    pub const fn bytes(&self) -> usize {
        self.bytes
    }
}

#[derive(Clone, Debug)]
pub struct UiTextureCatalog {
    pages: Arc<[UiTexturePage]>,
    plan: UiTexturePlan,
    dynamic_start: usize,
    identity: [u8; 32],
    static_identity: [u8; 32],
    source_identity: [u8; 32],
}

impl PartialEq for UiTextureCatalog {
    fn eq(&self, other: &Self) -> bool {
        self.identity == other.identity
            && self.static_identity == other.static_identity
            && self.plan == other.plan
            && self.dynamic_start == other.dynamic_start
    }
}
impl Eq for UiTextureCatalog {}

impl UiTextureCatalog {
    pub fn new(
        pages: Vec<UiTexturePage>,
        dynamic_start: usize,
    ) -> Result<Self, UiRenderRejectReason> {
        Self::with_source_identity(pages, dynamic_start, [0; 32])
    }

    /// Source identity is an invalidation namespace, never proof of pixels or
    /// budget admission. Production derives it from its loaded asset catalogs.
    pub fn with_source_identity(
        pages: Vec<UiTexturePage>,
        dynamic_start: usize,
        source_identity: [u8; 32],
    ) -> Result<Self, UiRenderRejectReason> {
        if dynamic_start > pages.len()
            || pages.is_empty()
            || pages.len() > MAX_UI_TEXTURE_LAYERS as usize
        {
            return Err(UiRenderRejectReason::InvalidTextureExtent);
        }
        if pages.len() - dynamic_start > MAX_UI_DYNAMIC_PAGES
            || pages[dynamic_start..].iter().any(|page| {
                page.dimensions != [256, 256] || matches!(&page.pixels, Pixels::Font { .. })
            })
        {
            return Err(UiRenderRejectReason::InvalidTextureExtent);
        }
        let dimensions = pages
            .iter()
            .map(UiTexturePage::dimensions)
            .collect::<Vec<_>>();
        let plan = UiTexturePlan::new(&dimensions)?;
        let mut all = Sha256::new();
        let mut static_pages = Sha256::new();
        all.update(source_identity);
        static_pages.update(source_identity);
        all.update((dynamic_start as u64).to_le_bytes());
        static_pages.update((dynamic_start as u64).to_le_bytes());
        for (index, page) in pages.iter().enumerate() {
            for side in page.dimensions {
                all.update(side.to_le_bytes());
                static_pages.update(side.to_le_bytes());
            }
            all.update(page.identity);
            if index < dynamic_start {
                static_pages.update(page.identity);
            }
        }
        Ok(Self {
            pages: pages.into(),
            plan,
            dynamic_start,
            identity: all.finalize().into(),
            static_identity: static_pages.finalize().into(),
            source_identity,
        })
    }

    pub fn replace_dynamic(
        &self,
        replacement: Vec<UiTexturePage>,
    ) -> Result<Self, UiRenderRejectReason> {
        if replacement.len() != self.pages.len() - self.dynamic_start
            || replacement
                .iter()
                .zip(&self.pages[self.dynamic_start..])
                .any(|(a, b)| a.dimensions != b.dimensions)
        {
            return Err(UiRenderRejectReason::InvalidTextureExtent);
        }
        let mut pages = self.pages[..self.dynamic_start].to_vec();
        pages.extend(replacement);
        Self::with_source_identity(pages, self.dynamic_start, self.source_identity)
    }
    pub fn pages(&self) -> &[UiTexturePage] {
        &self.pages
    }
    pub fn plan(&self) -> &UiTexturePlan {
        &self.plan
    }
    pub const fn identity(&self) -> [u8; 32] {
        self.identity
    }
    pub const fn static_identity(&self) -> [u8; 32] {
        self.static_identity
    }
    pub const fn dynamic_start(&self) -> usize {
        self.dynamic_start
    }
}
