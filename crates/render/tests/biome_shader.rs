use std::{fs, path::PathBuf};

fn shader(name: &str) -> String {
    fs::read_to_string(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("src")
            .join(name),
    )
    .unwrap_or_else(|error| panic!("read {name}: {error}"))
}

#[test]
fn shared_shader_uses_lattice_kernel_and_uniform_fast_path() {
    let source = shader("biome_tint.wgsl");

    assert!(source.contains("BIOME_LATTICE_STEP: i32 = 4"));
    assert!(source.contains("fn lattice_tap_weights(t: f32)"));
    assert!(source.contains("for (var tz = 0; tz < 4; tz += 1)"));
    assert!(source.contains("for (var tx = 0; tx < 4; tx += 1)"));
    assert!(source.contains("if (uniform_tint != 0xffffffffu)"));
    assert!(source.contains("coordinate.x = clamp(coordinate.x, 0, 15)"));
    assert!(source.contains("coordinate.z = clamp(coordinate.z, 0, 15)"));
}

#[test]
fn every_tinted_pipeline_calls_the_shared_blender() {
    for name in ["chunk.wgsl", "model.wgsl", "liquid.wgsl"] {
        let source = shader(name);
        assert!(
            source.contains("#import cinnabar::biome_tint"),
            "{name} must import the common biome contract"
        );
        assert!(
            source.contains("blended_biome_tint("),
            "{name} must apply the same blending kernel"
        );
    }
}

#[test]
fn special_foliage_bypasses_neighbour_average_but_uses_live_center_rule() {
    let source = shader("biome_tint.wgsl");
    assert!(source.contains("fn special_foliage_tint("));
    assert!(source.contains("case 0x200u: { return unpack_linear_rgb10(tint.birch); }"));
    assert!(source.contains("case 0x400u: { return unpack_linear_rgb10(tint.evergreen); }"));
    assert!(source.contains("case 0x600u: { return unpack_linear_rgb10(tint.dry_foliage); }"));
}
