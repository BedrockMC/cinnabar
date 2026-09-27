use asset_compiler::{
    GlyphAdvances, OutlineFontConfig, compile_outline_font, compile_outline_font_with_fallback,
};
use std::{fs, path::Path};

#[test]
#[ignore = "requires both explicitly fetched pinned outline sources"]
fn two_provider_carrier_is_deterministic_and_preserves_primary_page_and_metrics() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let manifest = fs::read(root.join("assets/ui-font-source.json")).unwrap();
    let source: serde_json::Value = serde_json::from_slice(&manifest).unwrap();
    let path = |prefix: &str| {
        root.join(".local/assets/ui-font")
            .join(source[format!("{prefix}commit")].as_str().unwrap())
            .join(source[format!("{prefix}font_file")].as_str().unwrap())
    };
    let primary_path = path("");
    let fallback_path = path("fallback_");
    let primary = fs::read(&primary_path).unwrap();
    let fallback = fs::read(&fallback_path).unwrap();
    let identity = assets::canonical_source_manifest_sha256(&manifest);
    let config = OutlineFontConfig {
        advances: GlyphAdvances::InkPlusGap {
            gap_px: 2,
            blank_advance_px: Some(8),
        },
        ..OutlineFontConfig::default()
    };
    let original = compile_outline_font(&primary_path, &primary, identity, config).unwrap();
    let merged = compile_outline_font_with_fallback(
        &primary_path,
        &primary,
        &fallback_path,
        &fallback,
        identity,
        config,
    )
    .unwrap();
    let repeat = compile_outline_font_with_fallback(
        &primary_path,
        &primary,
        &fallback_path,
        &fallback,
        identity,
        config,
    )
    .unwrap();
    assert_eq!(merged.bytes, repeat.bytes);
    let old = assets::RuntimeFontCatalog::decode(&original.bytes, identity).unwrap();
    let new = assets::RuntimeFontCatalog::decode(&merged.bytes, identity).unwrap();
    assert_eq!(old.pages()[0].rgba8, new.pages()[0].rgba8);
    for glyph in old.glyphs() {
        assert_eq!(Some(glyph), new.glyph(glyph.codepoint));
    }
    assert!(new.pages().len() <= 4);
    for page in &new.pages()[1..] {
        assert_eq!(page.source_bytes as usize, fallback.len());
    }
}

#[test]
#[ignore = "requires the explicitly built, hash-bound local font carrier"]
fn compiled_ui_font_covers_declared_native_samples() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let bytes = fs::read(root.join(".local/assets/compiled/ui-monocraft-v1.mcbefont"))
        .expect("build the explicit local font qualification carrier");
    let manifest = fs::read(root.join("assets/ui-font-source.json")).unwrap();
    let identity = assets::canonical_source_manifest_sha256(&manifest);
    let catalog = assets::RuntimeFontCatalog::decode(&bytes, identity).unwrap();
    for codepoint in ['\u{2713}', '\u{4e16}', '\u{754c}', '\u{7b2c}', '\u{4e8c}'] {
        let glyph = catalog.glyph(codepoint).unwrap_or_else(|| {
            panic!(
                "required U+{:04X} is absent from the carrier",
                u32::from(codepoint)
            )
        });
        assert!(glyph.advance_64 > 0);
        assert!(glyph.uv[0] < glyph.uv[2] && glyph.uv[1] < glyph.uv[3]);
    }
}
