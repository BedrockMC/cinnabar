use assets::{BIOME_RULE_FLAG_GRASS_SHADED, BiomeRule, CompiledBiomeAssets, TintMapId, TintSource};

/// Builds a constant-byte palette so the shading operation is independent of climate lookup.
fn fixture(grass: TintSource) -> CompiledBiomeAssets {
    let mut assets = CompiledBiomeAssets::diagnostic();
    for pixel in assets.tint_maps_rgb8.chunks_exact_mut(3) {
        pixel.copy_from_slice(&[100, 200, 40]);
    }
    assets.rules = vec![BiomeRule {
        id: 1,
        name: "test:shaded".into(),
        flags: BIOME_RULE_FLAG_GRASS_SHADED,
        grass,
        foliage: TintSource::map(TintMapId::Foliage),
        dry_foliage: TintSource::map(TintMapId::DryFoliage),
        water: TintSource::direct(0x617b64),
        temperature_bits: 0.5f32.to_bits(),
        downfall_bits: 0.5f32.to_bits(),
    }]
    .into_boxed_slice();
    assets
}

/// Converts a linear result back to the original byte for exact packed-color assertions.
fn bytes(color: [f32; 4]) -> [u8; 3] {
    std::array::from_fn(|i| {
        let c = color[i];
        ((if c <= 0.0031308 {
            c * 12.92
        } else {
            1.055 * c.powf(1.0 / 2.4) - 0.055
        }) * 255.0)
            .round() as u8
    })
}

#[test]
fn shaded_default_grass_uses_reference_packed_byte_transform() {
    let resolved = fixture(TintSource::map(TintMapId::Grass))
        .resolve_live(&[])
        .unwrap();
    assert_eq!(bytes(resolved.records[1].grass), [70, 126, 25]);
}

#[test]
fn shaded_flag_does_not_change_custom_grass_override() {
    let resolved = fixture(TintSource::direct(0x64c828))
        .resolve_live(&[])
        .unwrap();
    assert_eq!(bytes(resolved.records[1].grass), [100, 200, 40]);
}
