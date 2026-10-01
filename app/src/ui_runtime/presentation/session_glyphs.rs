//! Server glyph sheets (`font/glyph_XX.png`), packed into the trailing dynamic pages and
//! layered over the base font for every overridden code point.

use std::sync::Arc;

use assets::{CellGlyph, pack_cells};
use render::UiTexturePage;

use super::{UiPresentationRuntime, dynamic_textures};

const PAGE_SIDE: u32 = 256;
/// Dynamic-page offset of the first glyph page, after the ten general pages.
const FIRST_GLYPH_PAGE: usize = 10;

/// The session's glyph cells, cropped from the winning sheets.
#[derive(Debug, Default)]
pub(crate) struct SessionGlyphSheets {
    pub(crate) cells: Vec<CellGlyph>,
    pub(crate) prepared: std::sync::OnceLock<PreparedGlyphs>,
}

#[derive(Debug)]
pub(crate) struct PreparedGlyphs {
    pages: Vec<UiTexturePage>,
    glyphs: Vec<assets::SheetGlyph>,
}

impl SessionGlyphSheets {
    /// Packs optional glyphs on the compilation worker before publication.
    pub(crate) fn new(cells: Vec<CellGlyph>) -> Self {
        let sheets = Self {
            cells,
            prepared: Default::default(),
        };
        let _ = sheets.prepared();
        sheets
    }

    /// Keeps relative atlas pages reusable independently of the carrier page offset.
    fn prepared(&self) -> &PreparedGlyphs {
        self.prepared.get_or_init(|| {
            let atlas = pack_cells(&self.cells, 0, PAGE_SIDE, dynamic_textures::GLYPH_PAGES);
            PreparedGlyphs {
                glyphs: atlas.glyphs,
                pages: atlas
                    .pages
                    .into_iter()
                    .filter_map(|pixels| UiTexturePage::owned([PAGE_SIDE; 2], pixels.into()).ok())
                    .collect(),
            }
        })
    }
}

/// The packed pages for the sheets last seen on the UI runtime.
#[derive(Default)]
pub(super) struct SessionGlyphPages {
    source: Option<Arc<SessionGlyphSheets>>,
    pub(super) pages: Vec<UiTexturePage>,
}

/// Repacks the atlas and swaps the layout font when the runtime's sheet set changes identity.
pub(super) fn observe(
    runtime: &mut UiPresentationRuntime,
    sheets: Option<&Arc<SessionGlyphSheets>>,
) {
    let unchanged = match (&runtime.session_glyphs.source, sheets) {
        (Some(current), Some(next)) => Arc::ptr_eq(current, next),
        (None, None) => true,
        _ => false,
    };
    if unchanged {
        return;
    }
    let first_page = runtime.textures.dynamic_start() + FIRST_GLYPH_PAGE;
    let prepared = sheets.map(|sheets| sheets.prepared());
    runtime.font = match prepared {
        Some(atlas) if !atlas.glyphs.is_empty() => {
            let glyphs = atlas
                .glyphs
                .iter()
                .map(|glyph| {
                    let mut glyph = *glyph;
                    glyph.metrics.page += first_page as u16;
                    glyph
                })
                .collect::<Vec<_>>();
            Arc::new(runtime.base_font.with_glyphs(&glyphs, |_| true))
        }
        _ => Arc::clone(&runtime.base_font),
    };
    let pages = prepared
        .map(|atlas| atlas.pages.clone())
        .unwrap_or_default();
    runtime.session_glyphs = SessionGlyphPages {
        source: sheets.cloned(),
        pages,
    };
    dynamic_textures::rebuild(runtime);
}

impl SessionGlyphPages {
    /// Texels of UI page `page` when it is one of these glyph pages.
    pub(super) fn page(
        &self,
        runtime_dynamic_start: usize,
        page: usize,
    ) -> Option<super::nametag_atlas::GlyphPage<'_>> {
        let page = self
            .pages
            .get(page.checked_sub(runtime_dynamic_start + FIRST_GLYPH_PAGE)?)?;
        Some(super::nametag_atlas::GlyphPage {
            width: PAGE_SIDE,
            height: PAGE_SIDE,
            rgba8: page.pixels(),
        })
    }
}
