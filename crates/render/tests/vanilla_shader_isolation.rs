//! Frozen vanilla shader bytes, including RGB lighting, weighted variants and actor/item fog.
use sha2::{Digest, Sha256};

/// Removes Enhanced blocks and checkout-specific CRLF endings, preserving other vanilla bytes.
fn vanilla_source(source: &str) -> String {
    let source = source.replace("\r\n", "\n");
    let mut active = vec![true];
    let mut result = String::new();
    for line in source.split_inclusive('\n') {
        let directive = line.trim();
        if directive.starts_with("#ifdef ENHANCED") {
            active.push(false);
        } else if directive == "#else" && active.len() > 1 {
            let last = active.last_mut().unwrap();
            *last = !*last;
        } else if directive == "#endif" && active.len() > 1 {
            active.pop();
        } else if active.iter().all(|value| *value) {
            result.push_str(line);
        }
    }
    assert_eq!(active.len(), 1);
    result
}

#[test]
fn vanilla_shader_hash_ignores_checkout_line_endings() {
    let source = "  vanilla  \n#ifdef ENHANCED\n enhanced\n#else\n fallback\n#endif\n";
    let expected = "  vanilla  \n fallback\n";
    assert_eq!(vanilla_source(source), expected);
    assert_eq!(vanilla_source(&source.replace('\n', "\r\n")), expected);
}

#[test]
fn disabled_enhanced_preserves_vanilla_shader_bytes() {
    for (source, digest) in [
        (
            include_str!("../src/chunk.wgsl"),
            // Native cube axes, leaf sampler/metadata, and gamma-space cube
            // lighting with grass's opaque alpha tint mask. Model/carried
            // routes remain independently frozen.
            "6af5d25966db1647327b23d8027ce5b92ece63c174dd69919be6da5cd7802d7d",
        ),
        (
            include_str!("../src/model.wgsl"),
            "717662986ec8c5e089f9e4be585244ae0c99002ab772dda0b324646cf63a8dce",
        ),
        (
            include_str!("../src/liquid.wgsl"),
            "315e9b9d1f19b0e1889bebdbf340b986215e190c1c62c3f72948b48ca6545f49",
        ),
        (
            include_str!("../src/lighting.wgsl"),
            "a57357556effce8395bfcb8cec7cdd8e13f8b8683ae98c6ed074175dd87cc151",
        ),
        (
            include_str!("../src/biome_tint.wgsl"),
            // Native world seasonal colours retain doubled, unclamped palette
            // channels; particle clamping and shelter are tested separately.
            // Retains upstream's safe coordinate clamp after dev integration.
            "4a3d38c487e7325cc33218eb4c8e35c717b5c4637238a5d6a6ad5be2361c9623",
        ),
        (
            include_str!("../src/atmosphere.wgsl"),
            // Current native sky fan, celestial quad basis/diameters, texture
            // alpha and rain scaling, and premultiplied star fragment colour.
            "81d1f8217997284d303fccb3122b2d279ff7aa3dbf70e5f0c1190ddbf7f53edd",
        ),
        (
            include_str!("../src/material.wgsl"),
            "fb27c8813ac42c9f6bb903984fa1d04d017b4aa1d7e00685d16fea16721d3385",
        ),
        (
            include_str!("../src/actor.wgsl"),
            // Native color-mask rasters have their own source-pinned material flag. The
            // neutral/player branches and Enhanced isolation remain separately tested.
            // Dev's shared fog varying coexists with the three native samplers.
            "2490f0b75bedbdd9c98b8f81b729f93e0fd64db954afe47f425d8475769004b9",
        ),
        (
            include_str!("../src/dropped_item.wgsl"),
            "00032bfb50771ce9cb799276bff395deff386d1a06d899bca0ddf1ac3a4f23c2",
        ),
        (
            include_str!("../src/hand_rig.wgsl"),
            "f1206972ae2b5a81fe8df9f4308b498942e385d089cdb21576adba8871dc6190",
        ),
    ] {
        assert_eq!(
            format!("{:x}", Sha256::digest(vanilla_source(source))),
            digest
        );
    }
}

/// Freeze the real base descriptors as well as testing specialization mutations.
#[test]
fn vanilla_base_pipeline_construction_matches_baseline() {
    let source = include_str!("../src/chunk/pipeline/layouts.rs");
    let start = source
        .find("        let descriptor = RenderPipelineDescriptor")
        .unwrap();
    let end = source[start..]
        .find("\n#[derive(Clone, Copy, PartialEq")
        .unwrap()
        + start;
    let construction: String = source[start..end]
        .chars()
        .filter(|value| !value.is_whitespace())
        .collect();
    assert_eq!(
        format!("{:x}", Sha256::digest(construction)),
        // Raster no-cull is gated per cube material in the fragment; models
        // and liquids retain their existing policies and depth behavior.
        "eda4c533622d26744412404364d804c464b86edee4e2c1ea6986a9b04162f7bc"
    );
}
