//! Item tooltip text: name, enchantments and lore.

use protocol::{NetworkItemStack, item_display};

use super::UiRuntime;
use super::hud_layout::TooltipLine;

const NAME_COLOR: [u8; 4] = [255, 255, 255, 255];
const ENCHANT_COLOR: [u8; 4] = [170, 170, 170, 255];
const LORE_COLOR: [u8; 4] = [170, 0, 170, 255];

/// The language key suffix of an enchantment id, in protocol order.
const fn enchantment_key(id: i16) -> Option<&'static str> {
    Some(match id {
        0 => "protect.all",
        1 => "protect.fire",
        2 => "protect.fall",
        3 => "protect.explosion",
        4 => "protect.projectile",
        5 => "thorns",
        6 => "oxygen",
        7 => "waterWalker",
        8 => "waterWorker",
        9 => "damage.all",
        10 => "damage.undead",
        11 => "damage.arthropods",
        12 => "knockback",
        13 => "fire",
        14 => "lootBonus",
        15 => "digging",
        16 => "untouching",
        17 => "durability",
        18 => "lootBonusDigger",
        19 => "arrowDamage",
        20 => "arrowKnockback",
        21 => "arrowFire",
        22 => "arrowInfinite",
        23 => "lootBonusFishing",
        24 => "fishingSpeed",
        25 => "frostwalker",
        26 => "mending",
        27 => "curse.binding",
        28 => "curse.vanishing",
        29 => "tridentImpaling",
        30 => "tridentRiptide",
        31 => "tridentLoyalty",
        32 => "tridentChanneling",
        33 => "crossbowMultishot",
        34 => "crossbowPiercing",
        35 => "crossbowQuickCharge",
        36 => "soul_speed",
        37 => "swift_sneak",
        38 => "wind_burst",
        39 => "density",
        40 => "breach",
        41 => "lunge",
        _ => return None,
    })
}

fn level_text(runtime: &UiRuntime, level: u8) -> String {
    runtime
        .translation(&format!("enchantment.level.{level}"))
        .map_or_else(|| level.to_string(), |text| text.to_string())
}

/// The tooltip for one stack; a server-stated name wins over the item's own.
pub(super) fn tooltip_lines(
    runtime: &UiRuntime,
    stack: &NetworkItemStack,
    identifier: Option<&str>,
    stated_name: Option<&str>,
) -> Vec<TooltipLine> {
    let display = item_display(&stack.extra_data);
    let name = stated_name
        .map(str::to_owned)
        .or_else(|| display.name.as_deref().map(str::to_owned))
        .or_else(|| identifier.map(|id| runtime.localized_item_name(id)))
        .unwrap_or_else(|| "Unknown Item".to_owned());
    let mut lines = vec![TooltipLine {
        text: name,
        color: NAME_COLOR,
    }];
    for (id, level) in &display.enchantments {
        let label = enchantment_key(*id)
            .and_then(|key| runtime.translation(&format!("enchantment.{key}")))
            .map_or_else(|| format!("Enchantment {id}"), |text| text.to_string());
        lines.push(TooltipLine {
            text: format!("{label} {}", level_text(runtime, *level)),
            color: ENCHANT_COLOR,
        });
    }
    lines.extend(display.lore.iter().map(|line| TooltipLine {
        text: line.to_string(),
        color: LORE_COLOR,
    }));
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_protocol_enchantment_has_a_key() {
        assert!((0..=41).all(|id| enchantment_key(id).is_some()));
        assert!(enchantment_key(42).is_none());
    }
}
