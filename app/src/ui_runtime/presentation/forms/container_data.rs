//! Station bindings the vanilla container screens read beyond their item
//! cells: progress ratios, empty-slot art, and mount grid shapes. A bound
//! `#clip_ratio` is the fraction clipped away, so a full bar binds `0`.

use json_ui::{CollectionItem, DataSource, Scalar};
use protocol::WindowKind;

use crate::ui_runtime::UiRuntime;

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
