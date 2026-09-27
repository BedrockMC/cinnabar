use super::*;

fn ticks(identifier: &str, conditions: DestroyConditions) -> Option<u32> {
    let block = block_destroy_info(identifier).expect("known block");
    destroy_progress_per_tick(&block, &conditions).map(|rate| (1.0 / rate).ceil() as u32)
}

fn grounded(tool: Option<&str>) -> DestroyConditions {
    DestroyConditions {
        tool: tool.map(|identifier| HeldTool::from_identifier(identifier).expect("tool")),
        on_ground: true,
        ..DestroyConditions::default()
    }
}

#[test]
fn generated_table_carries_bedrock_hardness() {
    let stone = block_destroy_info("minecraft:stone").unwrap();
    assert_eq!(stone.hardness, 1.5);
    // Bedrock obsidian differs from other editions.
    assert_eq!(
        block_destroy_info("minecraft:obsidian").unwrap().hardness,
        35.0
    );
    assert!(block_destroy_info("minecraft:bedrock").unwrap().hardness < 0.0);
    assert_eq!(block_destroy_info("minecraft:not_a_block"), None);
    assert!(table().windows(2).all(|pair| pair[0].0 < pair[1].0));
}

#[test]
fn hand_and_tier_rates_match_documented_break_times() {
    assert_eq!(ticks("minecraft:dirt", grounded(None)), Some(15));
    assert_eq!(ticks("minecraft:stone", grounded(None)), Some(150));
    assert_eq!(
        ticks(
            "minecraft:stone",
            grounded(Some("minecraft:wooden_pickaxe"))
        ),
        Some(23)
    );
    assert_eq!(
        ticks("minecraft:oak_log", grounded(Some("minecraft:stone_axe"))),
        Some(15)
    );
    // A pickaxe is not an axe: log speed stays at the hand rate.
    assert_eq!(
        ticks(
            "minecraft:oak_log",
            grounded(Some("minecraft:diamond_pickaxe"))
        ),
        Some(60)
    );
}

#[test]
fn harvest_tier_selects_the_slow_divisor() {
    let iron = ticks(
        "minecraft:obsidian",
        grounded(Some("minecraft:iron_pickaxe")),
    )
    .unwrap();
    let diamond = ticks(
        "minecraft:obsidian",
        grounded(Some("minecraft:diamond_pickaxe")),
    )
    .unwrap();
    assert_eq!(diamond, 132);
    assert!(
        iron > diamond * 3,
        "iron {iron} must use the unharvestable divisor"
    );
    assert_eq!(
        ticks("minecraft:web", grounded(Some("minecraft:shears"))),
        Some(8)
    );
    assert_eq!(ticks("minecraft:web", grounded(None)), Some(400));
}

#[test]
fn environment_and_effects_scale_speed() {
    let base = grounded(Some("minecraft:wooden_pickaxe"));
    let airborne = DestroyConditions {
        on_ground: false,
        ..base
    };
    assert_eq!(ticks("minecraft:stone", airborne), Some(113));
    let submerged = DestroyConditions {
        eyes_in_water: true,
        ..base
    };
    assert_eq!(ticks("minecraft:stone", submerged), Some(113));
    let aqua = DestroyConditions {
        aqua_affinity: true,
        ..submerged
    };
    assert_eq!(ticks("minecraft:stone", aqua), Some(23));
    let haste = DestroyConditions {
        haste_amplifier: Some(1),
        ..base
    };
    assert_eq!(ticks("minecraft:stone", haste), Some(12));
    let fatigue = DestroyConditions {
        mining_fatigue_amplifier: Some(0),
        ..base
    };
    assert_eq!(ticks("minecraft:stone", fatigue), Some(108));
    let riding = DestroyConditions {
        riding: true,
        ..base
    };
    assert_eq!(ticks("minecraft:stone", riding), Some(113));
    let odd = DestroyConditions {
        mining_fatigue_amplifier: Some(-3),
        ..base
    };
    assert!(ticks("minecraft:stone", odd).unwrap() > 2_000);
}

#[test]
fn zero_hardness_is_instant_and_negative_is_indestructible() {
    assert_eq!(ticks("minecraft:torch", grounded(None)), Some(1));
    assert_eq!(
        ticks(
            "minecraft:bedrock",
            grounded(Some("minecraft:netherite_pickaxe"))
        ),
        None
    );
}

#[test]
fn only_vanilla_tool_identifiers_classify() {
    assert_eq!(
        HeldTool::from_identifier("minecraft:golden_hoe"),
        Some(HeldTool {
            kind: ToolKind::Hoe,
            tier: Some(ToolTier::Gold)
        })
    );
    for identifier in [
        "minecraft:stick",
        "minecraft:iron_shears",
        "custom:iron_pickaxe",
        "minecraft:iron_ingot",
    ] {
        assert_eq!(HeldTool::from_identifier(identifier), None, "{identifier}");
    }
}

#[test]
fn table_header_pins_manifest_sources() {
    let manifest = include_str!("../../../../assets/block-data-sources.json");
    let pins = DESTROY_TABLE
        .lines()
        .take_while(|line| line.starts_with('#'))
        .filter_map(|line| line.split_once("sha256=").map(|(_, digest)| digest))
        .collect::<Vec<_>>();
    assert_eq!(pins.len(), 2);
    for digest in pins {
        assert!(
            manifest.contains(&format!("\"sha256\": \"{digest}\"")),
            "{digest}"
        );
    }
}
