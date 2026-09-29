//! Session glyph sheets from the stack's `font/glyph_XX.png`; the highest pack's sheet wins.

use std::sync::Arc;

use assets::GlyphSheet;
use resource_pack::LayeredPackView;

use super::resource_packs::decode_pack_texture;
use crate::ui_runtime::presentation::SessionGlyphSheets;

/// Decodes every sheet present in the stack, or `None` when it has none.
pub(super) fn compile_session_glyphs(view: &LayeredPackView) -> Option<Arc<SessionGlyphSheets>> {
    let sheets = (0..=u8::MAX)
        .filter_map(|high_byte| {
            let texture = [
                format!("font/glyph_{high_byte:02X}"),
                format!("font/glyph_{high_byte:02x}"),
            ]
            .iter()
            .find_map(|path| decode_pack_texture(view, path))?;
            Some(GlyphSheet {
                high_byte,
                width: texture.width,
                height: texture.height,
                rgba8: texture.rgba8,
            })
        })
        .collect::<Vec<_>>();
    (!sheets.is_empty()).then(|| Arc::new(SessionGlyphSheets { sheets }))
}

#[cfg(test)]
mod tests;
