//! Server glyph sheets (`font/glyph_XX.png`), packed into the trailing dynamic pages and
//! layered over the base font so private-use code points draw the pack's art.

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
    let atlas = sheets.map(|sheets| {
        pack_cells(
            &sheets.cells,
            first_page as u16,
            PAGE_SIDE,
            dynamic_textures::GLYPH_PAGES,
        )
    });
    runtime.font = match &atlas {
        Some(atlas) if !atlas.glyphs.is_empty() => Arc::new(
            runtime
                .base_font
                .with_glyphs(&atlas.glyphs, |c| ('\u{e000}'..='\u{f8ff}').contains(&c)),
        ),
        _ => Arc::clone(&runtime.base_font),
    };
    let pages = atlas
        .into_iter()
        .flat_map(|atlas| atlas.pages)
        .filter_map(|rgba8| UiTexturePage::owned([PAGE_SIDE, PAGE_SIDE], rgba8.into()).ok())
        .collect();
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
