//! Station bindings the vanilla container screens read beyond their item
//! cells: progress ratios, empty-slot art, mount grid shapes, and the station
//! controls (enchant options, the anvil name, stonecutter recipes, beacon
//! powers) with the widgets their hit regions press. A bound `#clip_ratio` is
//! the fraction clipped away, so a full bar binds `0`.

use json_ui::{CollectionItem, DataSource, HitKind, HitRegion, Scalar};
use protocol::WindowKind;

use super::super::{HudFrame, IconRef};
use crate::ui_runtime::UiRuntime;
use crate::ui_runtime::inventory_ledger::InventoryTarget;
use crate::ui_runtime::presentation::screens::{BEACON_LEVEL_FOR, STONECUTTER_CELLS, Widget};

/// Beacon power buttons by collection: `(name, effect id, secondary)`.
const BEACON_POWERS: [(&str, i32, bool); 6] = [
    ("speed", 1, false),
    ("haste", 3, false),
    ("resist", 11, false),
    ("jump", 8, false),
    ("strength", 5, false),
    ("regen", 10, true),
];
/// The legacy `id << 16 | aux` values the beacon's payment row names.
const BEACON_PAYMENTS: [(i64, &str); 5] = [
    (742 << 16, "minecraft:netherite_ingot"),
    (388 << 16, "minecraft:emerald"),
    (264 << 16, "minecraft:diamond"),
    (266 << 16, "minecraft:gold_ingot"),
    (265 << 16, "minecraft:iron_ingot"),
];
const CELL_NORMAL: &str = "textures/ui/cell_image_normal";
const CELL_SELECTED: &str = "textures/ui/cell_image_invert";

const FURNACE_COOK_TICKS: f64 = 200.0;
const FAST_COOK_TICKS: f64 = 100.0;
const BREW_TICKS: f64 = 400.0;
const DEFAULT_FUEL_TOTAL: f64 = 20.0;
/// Bubble heights of the brewing cycle, of the 29 px column (provisional: Java's cycle).
const BUBBLE_HEIGHTS: [f64; 7] = [29.0, 24.0, 20.0, 16.0, 11.0, 6.0, 0.0];

/// Globals for the open station's progress and layout.
pub(super) fn station_globals(data: &mut DataSource, runtime: &UiRuntime, kind: WindowKind) {
    let ledger = runtime.inventory_ledger();
    let property = |id: i32| ledger.window_data(id).map(f64::from);
    let mut clip = |name: &str, shown: f64| {
        data.set_global(name, Scalar::Num(1.0 - shown.clamp(0.0, 1.0)));
    };
    match kind {
        WindowKind::Furnace | WindowKind::BlastFurnace | WindowKind::Smoker => {
            let total = if kind == WindowKind::Furnace {
                FURNACE_COOK_TICKS
            } else {
                FAST_COOK_TICKS
            };
            clip(
                "#furnace_arrow_ratio",
                property(0).map_or(0.0, |ticks| ticks / total),
            );
            let lit = match (property(1), property(2)) {
                (Some(remaining), Some(duration)) if duration > 0.0 => remaining / duration,
                _ => 0.0,
            };
            clip("#furnace_flame_ratio", lit);
        }
        WindowKind::Brewing => {
            let remaining = property(0).filter(|ticks| *ticks > 0.0);
            clip(
                "#brewing_arrow_ratio",
                remaining.map_or(0.0, |ticks| 1.0 - ticks / BREW_TICKS),
            );
            let bubbles = remaining.map_or(0.0, |ticks| {
                BUBBLE_HEIGHTS[(ticks as usize / 2) % BUBBLE_HEIGHTS.len()] / 29.0
            });
            clip("#brewing_bubbles_ratio", bubbles);
            let total = property(2)
                .filter(|total| *total > 0.0)
                .unwrap_or(DEFAULT_FUEL_TOTAL);
            clip(
                "#brewing_fuel_ratio",
                property(1).map_or(0.0, |fuel| fuel / total),
            );
        }
        WindowKind::Horse => {
            let chest = ledger.storage_slot_count().unwrap_or(2).saturating_sub(2);
            data.set_grid_dimensions("#equip_grid_dimensions", [1, 2]);
            data.set_grid_dimensions("#inv_grid_dimensions", [(chest / 3) as u32, 3]);
            data.set_global("#is_chested", Scalar::Bool(chest > 0));
            // The mount's kind is not tracked; every mount shows the horse's slots.
            data.set_global("#has_saddle_slot", Scalar::Bool(true));
            data.set_global("#has_horse_armor_and_saddle_slot", Scalar::Bool(true));
        }
        _ => {}
    }
}

/// Collections and globals of the open station's controls.
pub(super) fn station_controls(
    data: &mut DataSource,
    runtime: &UiRuntime,
    frame: &HudFrame,
    kind: WindowKind,
) {
    match kind {
        WindowKind::Enchanting => data.set_collection("#enchant_buttons", enchant_buttons(runtime)),
        WindowKind::Anvil => {
            let name = runtime.screen_state().anvil_name.clone();
            data.set_global("#text_box_item_name", Scalar::Text(name));
        }
        WindowKind::Stonecutter => data.set_collection("stones", stones(runtime, frame)),
        WindowKind::Beacon => beacon_buttons(data, runtime),
        WindowKind::Cartography => data.set_global("#is_none_mode", Scalar::Bool(true)),
        _ => {}
    }
}

/// Icons for `#item_id_aux` renderers: the beacon's payment row by its legacy
/// ids, and the stonecutter's recipes by the negative keys `stones` binds.
pub(super) fn id_aux_icons(
    runtime: &UiRuntime,
    frame: &HudFrame,
    icon: impl Fn(&str) -> Option<IconRef>,
) -> Vec<(i64, IconRef)> {
    match runtime.inventory_ledger().window_kind() {
        Some(WindowKind::Beacon) => BEACON_PAYMENTS
            .iter()
            .filter_map(|(key, id)| Some((*key, icon(id)?)))
            .collect(),
        Some(WindowKind::Stonecutter) => frame
            .window_icons
            .recipe
            .iter()
            .enumerate()
            .filter_map(|(index, icon)| Some((stone_key(index), (*icon)?)))
            .collect(),
        _ => Vec::new(),
    }
}

fn stone_key(index: usize) -> i64 {
    -1 - index as i64
}

/// The station widget a hit region presses, for regions that are not item cells.
pub(super) fn widget_hit(screen: &str, region: &HitRegion) -> Option<Widget> {
    let index = region.collection_index.unwrap_or(0);
    let collection = region.collection.as_deref();
    Some(match (screen, collection) {
        ("enchanting.enchanting_screen", Some("#enchant_buttons"))
            if region.name == "selectable_button" =>
        {
            Widget::EnchantOption(u8::try_from(index).ok()?)
        }
        ("anvil.anvil_screen", _) if region.kind == HitKind::EditBox => Widget::AnvilName,
        ("stonecutter.stonecutter_screen", Some("stones")) if index < STONECUTTER_CELLS => {
            Widget::StonecutterRecipe(u8::try_from(index).ok()?)
        }
        ("beacon.beacon_screen", Some("extra")) => Widget::BeaconUpgrade,
        ("beacon.beacon_screen", Some("confirm")) => Widget::BeaconConfirm,
        ("beacon.beacon_screen", Some(name)) => {
            let (_, id, secondary) = BEACON_POWERS.iter().find(|power| power.0 == name)?;
            Widget::BeaconEffect {
                id: *id,
                secondary: *secondary,
            }
        }
        _ => return None,
    })
}

/// The three option rows: selectable when the player has the levels and lapis
/// (creative always does), with the vanilla clue and cost hover text.
fn enchant_buttons(runtime: &UiRuntime) -> Vec<CollectionItem> {
    let ledger = runtime.inventory_ledger();
    let options = ledger.enchant_options().unwrap_or(&[]);
    let level = runtime.hud().experience().map_or(0, |xp| xp.level);
    let lapis = ledger
        .target_stack(InventoryTarget::Craft(15))
        .map_or(0, |stack| u32::from(stack.count));
    let creative = runtime.player_game_mode() == Some(protocol::PlayerGameMode::Creative);
    (0..3u32)
        .map(|row| {
            let item = CollectionItem::default();
            let Some(option) = options.get(row as usize) else {
                return item
                    .with("#selectable_button_visibility", Scalar::Bool(false))
                    .with("#unselectable_button_visibility", Scalar::Bool(false));
            };
            let has_levels = creative || level >= u32::from(option.cost);
            let has_lapis = creative || lapis > row;
            let selectable = has_levels && has_lapis;
            item.with("#selectable_button_visibility", Scalar::Bool(selectable))
                .with("#unselectable_button_visibility", Scalar::Bool(!selectable))
                .with("#selectable_dust_is_visible", Scalar::Bool(selectable))
                .with("#unselectable_dust_is_visible", Scalar::Bool(!selectable))
                .with("#cost", Scalar::Text(option.cost.to_string()))
                // The rune font is not carried; the galactic text stays unset.
                .with("#runes", Scalar::Text(String::new()))
                .with(
                    "#hover_text",
                    Scalar::Text(enchant_hover(
                        runtime,
                        option,
                        row + 1,
                        has_levels,
                        has_lapis,
                        creative,
                    )),
                )
        })
        .collect()
}

fn enchant_hover(
    runtime: &UiRuntime,
    option: &protocol::EnchantOption,
    levels: u32,
    has_levels: bool,
    has_lapis: bool,
    creative: bool,
) -> String {
    let text = |key: &str| {
        runtime
            .translation(key)
            .map_or_else(|| key.to_owned(), |text| text.to_string())
    };
    let clue = option
        .enchants
        .first()
        .map(|(id, level)| {
            let name =
                super::super::inventory_tooltip::enchantment_name(runtime, i16::from(*id), *level);
            text("container.enchant.clue").replacen("%s", &name, 1)
        })
        .unwrap_or_default();
    let mut lines = vec![clue];
    if !creative {
        if has_levels {
            let (lapis, level) = if levels == 1 {
                ("container.enchant.lapis.one", "container.enchant.level.one")
            } else {
                (
                    "container.enchant.lapis.many",
                    "container.enchant.level.many",
                )
            };
            let lapis_color = if has_lapis { "§7" } else { "§c" };
            lines.push(format!(
                "{lapis_color}{}",
                text(lapis).replacen("%d", &levels.to_string(), 1)
            ));
            lines.push(format!(
                "§7{}",
                text(level).replacen("%d", &levels.to_string(), 1)
            ));
        } else {
            let requirement = text("container.enchant.levelrequirement");
            lines.push(format!(
                "§c{}",
                requirement.replacen("%d", &option.cost.to_string(), 1)
            ));
        }
    }
    lines.join("\n")
}

/// The stonecutter's recipe cells for the input, the chosen one inverted.
fn stones(runtime: &UiRuntime, frame: &HudFrame) -> Vec<CollectionItem> {
    let chosen = runtime.active_screen_recipe().map(|recipe| recipe.id);
    let options = runtime.stonecutter_options();
    let total = options.len().min(STONECUTTER_CELLS);
    options
        .iter()
        .take(STONECUTTER_CELLS)
        .enumerate()
        .map(|(index, recipe)| {
            let output = recipe.output;
            let name = output
                .and_then(|output| {
                    frame
                        .item_names
                        .get(&(output.network_id, u32::from(output.aux)))
                })
                .map_or_else(String::new, |name| name.to_string());
            let count = output.map_or(0, |output| output.count);
            let texture = if chosen == Some(recipe.id) {
                CELL_SELECTED
            } else {
                CELL_NORMAL
            };
            CollectionItem::default()
                .with("#item_id_aux", Scalar::Num(stone_key(index) as f64))
                .with(
                    "#item_stack_count",
                    Scalar::Text(if count > 1 {
                        count.to_string()
                    } else {
                        String::new()
                    }),
                )
                .with(
                    "#stone_cell_background_texture",
                    Scalar::Text(texture.to_owned()),
                )
                .with("#stone_selector_total_items", Scalar::Num(total as f64))
                .with("#hover_text", Scalar::Text(name))
        })
        .collect()
}

/// One collection per beacon button, as the controller names them.
fn beacon_buttons(data: &mut DataSource, runtime: &UiRuntime) {
    let state = runtime.screen_state();
    let (primary, secondary) = state.beacon;
    let unlocked = |needed: u8| state.beacon_level.is_none_or(|have| have >= needed);
    let button = |active: bool, selected: bool, hover: String| {
        vec![
            CollectionItem::default()
                .with("#button_visible", Scalar::Bool(true))
                .with("#active", Scalar::Bool(active && !selected))
                .with("#inactive", Scalar::Bool(!active))
                .with("#selected", Scalar::Bool(active && selected))
                .with("#button_hover", Scalar::Text(hover)),
        ]
    };
    let name = |id: i32| {
        let key = match id {
            1 => "effect.moveSpeed",
            3 => "effect.digSpeed",
            11 => "effect.resistance",
            8 => "effect.jump",
            5 => "effect.damageBoost",
            _ => "effect.regeneration",
        };
        runtime
            .translation(key)
            .map_or_else(|| key.to_owned(), |text| text.to_string())
    };
    for (collection, id, is_secondary) in BEACON_POWERS {
        let needed = BEACON_LEVEL_FOR
            .iter()
            .find(|(effect, _)| *effect == id)
            .map_or(4, |(_, level)| *level);
        let selected = if is_secondary {
            secondary == id
        } else {
            primary == id
        };
        data.set_collection(collection, button(unlocked(needed), selected, name(id)));
    }
    let upgrade = primary != 0 && unlocked(4);
    data.set_collection(
        "extra",
        button(upgrade, upgrade && secondary == primary, name(primary)),
    );
    data.set_collection("confirm", button(primary != 0, false, String::new()));
    data.set_collection("cancel", button(true, false, String::new()));
}

/// How many cells of `collection` the open window fills; the mount chest shows
/// only the columns the mount carries.
pub(super) fn collection_len(runtime: &UiRuntime, collection: &str, cells: usize) -> usize {
    match (runtime.inventory_ledger().window_kind(), collection) {
        (Some(WindowKind::Horse), "container_items") => runtime
            .inventory_ledger()
            .storage_slot_count()
            .map_or(0, |count| count.saturating_sub(2))
            .min(cells),
        _ => cells,
    }
}

/// Empty-slot silhouettes the brewing stand binds per cell.
pub(super) fn decorate(collection: &str, empty: bool, item: CollectionItem) -> CollectionItem {
    match collection {
        "brewing_result_items" => item.with("#empty_bottle_image_visible", Scalar::Bool(empty)),
        "brewing_fuel_item" => item.with("#empty_fuel_image_visible", Scalar::Bool(empty)),
        _ => item,
    }
}
