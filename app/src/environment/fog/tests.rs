use super::*;
use assets::TintSource;

fn rule(id: u32, name: &str, downfall: f32) -> BiomeRule {
    BiomeRule {
        id,
        name: name.into(),
        flags: 0,
        grass: TintSource::direct(0),
        foliage: TintSource::direct(0),
        dry_foliage: TintSource::direct(0),
        water: TintSource::direct(0),
        temperature_bits: 0.0_f32.to_bits(),
        downfall_bits: downfall.to_bits(),
    }
}

fn definition(id: Option<u16>, name: &str, downfall: f32) -> BiomeDefinitionEvent {
    BiomeDefinitionEvent {
        biome_id: id,
        name: Arc::from(name),
        temperature: 0.0,
        downfall,
        snow_foliage: 0.0,
        max_snow_accumulation: None,
        map_water_color: 0,
    }
}

#[test]
fn weather_and_fog_use_one_floored_native_lattice() {
    let origin = [-1.25, 63.875, 2.5];
    let positions = sample_positions(origin);
    for (actual, offset) in positions.iter().zip(PRECIPITATION_SAMPLE_OFFSETS) {
        assert_eq!(
            *actual,
            std::array::from_fn(|axis| origin[axis].floor() + offset[axis] as f32)
        );
    }
    assert_eq!(positions.len(), PRECIPITATION_SAMPLE_OFFSETS.len());
}

#[test]
fn precipitation_admission_uses_native_epsilon_not_positive_downfall() {
    assert_eq!(precipitation_eligible(0.0), Some(false));
    assert_eq!(precipitation_eligible(f32::EPSILON * 0.5), Some(false));
    assert_eq!(precipitation_eligible(f32::EPSILON), Some(true));
    assert_eq!(precipitation_eligible(0.8), Some(true));
    for value in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
        assert_eq!(precipitation_eligible(value), None);
    }
}

#[test]
fn live_climate_overrides_canonical_ids_and_custom_ids_without_shadowing() {
    let rules = [
        rule(4, "minecraft:forest", 0.8),
        rule(8, "minecraft:desert", 0.0),
    ];
    let definitions = [
        definition(Some(91), "minecraft:forest", 0.0),
        definition(None, "minecraft:desert", 0.8),
        definition(Some(92), "custom:wet", 0.8),
        definition(Some(8), "custom:collision", 0.0),
        definition(None, "custom:unbound", 0.8),
        definition(Some(93), "custom:invalid", f32::NAN),
    ];
    let resolved = precipitation_registry(&rules, &definitions);
    assert_eq!(resolved.get(&4), Some(&Some(false)));
    assert_eq!(resolved.get(&8), Some(&Some(true)));
    assert_eq!(resolved.get(&92), Some(&Some(true)));
    assert_eq!(resolved.get(&93), Some(&None));
    assert!(!resolved.contains_key(&91));
    assert_eq!(resolved.len(), rules.len() + 2);
}

#[test]
fn missing_cells_never_get_renormalized_or_assigned_fallback_precipitation() {
    assert_eq!(count_precipitation_samples([None, None]), None);
    assert_eq!(count_precipitation_samples([Some(false), None]), Some(0));
    assert_eq!(
        count_precipitation_samples([Some(true), None, Some(false)]),
        Some(1)
    );
}
