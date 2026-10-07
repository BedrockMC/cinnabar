//! Bounded native fallback rasters, selected from the installed face priority chain.

use super::super::super::super::{FontCompileError, invalid};
use super::{NATIVE_SDF_EM_PIXELS, atlas, freetype, sdf};
use assets::{FontPixels, FontRendering, FontTexturePage, RuntimeFontCatalog, encode_font_catalog};
use sha2::{Digest, Sha256};

/// Rasterizes only requested Unicode scalars, keeping primary-face glyphs out of the atlas.
pub fn compile_native_fallback_fonts(
    sources: &[&[u8]],
    codepoints: &[char],
    atlas_side: u32,
    maximum_pages: usize,
) -> Result<RuntimeFontCatalog, FontCompileError> {
    if sources.is_empty()
        || sources.len() > 16
        || codepoints.len() > assets::MAX_FONT_GLYPHS
        || !atlas_side.is_power_of_two()
        || !(256..=assets::FONT_FALLBACK_ATLAS_SIDE).contains(&atlas_side)
        || maximum_pages == 0
        || maximum_pages > assets::MAX_FONT_FALLBACK_PAGES
        || sources
            .iter()
            .any(|s| s.is_empty() || s.len() as u64 > assets::MAX_FONT_SOURCE_BYTES)
        || sources
            .iter()
            .try_fold(0u64, |total, source| total.checked_add(source.len() as u64))
            .is_none_or(|total| total > assets::MAX_FONT_SOURCE_BYTES)
    {
        return Err(invalid("native fallback sources exceed bounds"));
    }
    let mut hash = Sha256::new();
    let mut faces = Vec::new();
    for source in sources {
        hash.update(Sha256::digest(source));
        faces.push(freetype::Face::new(source, NATIVE_SDF_EM_PIXELS)?);
    }
    let identity = hash.finalize().into();
    let line = faces[0].line_metrics()?;
    let mut seen = std::collections::BTreeSet::new();
    let mut glyphs = Vec::new();
    for &ch in codepoints {
        if !seen.insert(ch) {
            continue;
        }
        if let Some(face) = faces.iter_mut().find(|face| face.has(ch)) {
            glyphs.push(sdf::glyph(face.rasterize(ch)?)?);
        }
    }
    if glyphs.is_empty() {
        return Err(invalid("native fallback has no supported glyphs"));
    }
    let mut atlas = atlas::pack_pages(glyphs, atlas_side, false)?;
    if atlas.pages.len() > maximum_pages {
        return Err(invalid("native fallback atlas exceeds its page budget"));
    }
    atlas.glyphs.sort_unstable_by_key(|glyph| glyph.codepoint);
    let pages: Vec<_> = atlas
        .pages
        .into_iter()
        .enumerate()
        .map(|(index, pixels)| {
            let hash = Sha256::digest(&pixels).into();
            FontTexturePage {
                source_path: format!("font/runtime-fallback-{index}.png").into(),
                source_bytes: pixels.len() as u32,
                source_sha256: hash,
                pixels_sha256: hash,
                width: atlas_side,
                height: atlas_side,
                pixels: FontPixels::Rgba8(pixels),
            }
        })
        .collect();
    let bytes = encode_font_catalog(identity, &atlas.glyphs, &pages)?;
    Ok(RuntimeFontCatalog::decode(&bytes, identity)?
        .with_line_metrics(line)?
        .with_rendering(FontRendering::NativeSdf)
        .with_coverage_pages())
}
