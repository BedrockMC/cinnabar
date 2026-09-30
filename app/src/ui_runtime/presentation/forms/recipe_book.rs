//! The inventory and crafting table's recipe book panel: the creative catalog
//! in creative, the craftable recipes otherwise, filed under the vanilla tabs.
//! Tab and layout toggles press the existing screen widgets.

use json_ui::{CollectionItem, Context, DataSource, HitKind, HitRegion, Scalar};
use serde_json::Value;

use super::super::{HudFrame, IconRef};
use crate::ui_runtime::UiRuntime;
use crate::ui_runtime::inventory_actions::recipe_book_entries;
use crate::ui_runtime::presentation::inventory_pointer::InventoryCellHit;
use crate::ui_runtime::presentation::screens::{SEARCH_TAB, Widget};

/// `CraftingScreenController::addStaticScreenVars`: radio indexes of the tabs
/// and layout toggles.
const INDEXES: [(&str, u64); 9] = [
    ("construction_index", 1),
    ("equipment_index", 2),
    ("items_index", 3),
    ("nature_index", 4),
    ("search_index", 5),
    ("survival_index", 6),
    ("survival_layout_index", 1),
    ("recipe_book_layout_index", 2),
    ("creative_layout_index", 3),
];
/// Screen-state tabs by radio index: construction, equipment, items, nature, search.
const TABS: [(u64, u8); 5] = [(1, 0), (2, 2), (3, 3), (4, 1), (5, SEARCH_TAB)];
/// Tab labels by screen-state tab; the search tab's label is provisional.
const TAB_LABELS: [&str; 5] = [
    "craftingScreen.tab.construction",
    "craftingScreen.tab.nature",
    "craftingScreen.tab.equipment",
    "craftingScreen.tab.items",
    "craftingScreen.tab.allItems",
];
const ITEM_BACKGROUND: &str = "textures/ui/recipe_book_item_bg";

/// Whether the panel shows: creative opens on it and the toggle flips either way.
pub(crate) fn recipe_book_shown(runtime: &UiRuntime) -> bool {
    let creative = runtime.player_game_mode() == Some(protocol::PlayerGameMode::Creative);
    creative != runtime.screen_state().book_open
}

/// The crafting screens' static variables.
pub(super) fn context(mut context: Context) -> Context {
    for (name, index) in INDEXES {
        context = context.with_var(name, Value::from(index));
    }
    context
}

/// The layout, tab and search globals and the `recipe_book` collection.
pub(super) fn book_data(
    data: &mut DataSource,
    runtime: &UiRuntime,
    frame: &HudFrame,
    icons: &mut Vec<IconRef>,
    shown: bool,
) {
    let creative = runtime.player_game_mode() == Some(protocol::PlayerGameMode::Creative);
    let state = runtime.screen_state();
    let tab = state.creative_tab;
    for (name, value) in [
        ("#is_survival_layout", !shown),
        ("#is_recipe_book_layout", shown),
        ("#is_creative_mode", creative),
        ("#is_creative_layout_button_visible", creative),
        ("#is_creative_and_recipe_book_layout", creative && shown),
        // The survival book lists only what the player can craft now.
        ("#filtering_enabled", !creative),
        ("#is_left_tab_inventory", !shown),
        ("#construction_tab_visible", true),
        ("#equipment_tab_visible", true),
        ("#items_tab_visible", true),
        ("#nature_tab_visible", true),
        ("#is_left_tab_construct", tab == 0),
        ("#is_left_tab_nature", tab == 1),
        ("#is_left_tab_equipment", tab == 2),
        ("#is_left_tab_items", tab == 3),
        ("#is_left_tab_search", tab == SEARCH_TAB),
    ] {
        data.set_global(name, Scalar::Bool(value));
    }
    let layout = if shown { 2 } else { 1 };
    data.select_radio("layout_toggle", layout);
    if let Some((index, _)) = TABS.iter().find(|(_, java)| *java == tab) {
        data.select_radio("navigation_tab", *index as usize);
    }
    let label = TAB_LABELS[usize::from(tab.min(SEARCH_TAB))];
    let label = runtime
        .translation(label)
        .map_or_else(|| label.to_owned(), |text| text.to_string());
    data.set_global("#tab_label_text", Scalar::Text(label));
    data.set_global("#text_box_item_name", Scalar::Text(state.search.clone()));
    if !shown {
        return;
    }
    let entries = recipe_book_entries(runtime);
    let total = entries.len() as f64;
    let hovered = match state.hover {
        Some(InventoryCellHit::RecipeBook(index)) => Some(usize::from(index)),
        _ => None,
    };
    let items = entries
        .iter()
        .enumerate()
        .map(|(index, entry)| {
            let mut item = CollectionItem::default();
            if let Some(icon) = frame
                .window_icons
                .book_entries
                .get(index)
                .copied()
                .flatten()
            {
                icons.push(icon);
                item = item.with("#item_renderer_data", Scalar::Num((icons.len() - 1) as f64));
            }
            let count = entry.stack().count;
            // Only the hovered entry's text is ever read.
            let hover = if hovered == Some(index) {
                super::containers::tooltip_text(&frame.window_text.tooltip).unwrap_or_default()
            } else {
                String::new()
            };
            item.with(
                "#recipe_craftable_count",
                Scalar::Text(if count > 1 && !creative {
                    count.to_string()
                } else {
                    String::new()
                }),
            )
            .with("#recipe_hover_text", Scalar::Text(hover))
            .with("#is_creative_selected_slot", Scalar::Bool(false))
            .with(
                "#container_item_background_texture",
                Scalar::Text(ITEM_BACKGROUND.to_owned()),
            )
            .with("#recipe_book_total_items", Scalar::Num(total))
        })
        .collect();
    data.set_collection("recipe_book", items);
}

/// The widget a recipe book control presses: an entry, a tab, the search
/// field, or the layout toggle that flips the panel.
pub(super) fn book_hit(region: &HitRegion, shown: bool) -> Option<InventoryCellHit> {
    if region.collection.as_deref() == Some("recipe_book") {
        return Some(InventoryCellHit::RecipeBook(
            u16::try_from(region.collection_index?).ok()?,
        ));
    }
    if region.kind == HitKind::EditBox && shown {
        return Some(InventoryCellHit::CreativeSearch);
    }
    let group = region.group_index? as u64;
    match region.control_name.as_deref()? {
        "navigation_tab" => {
            let (_, tab) = TABS.iter().find(|(index, _)| *index == group)?;
            Some(if *tab == SEARCH_TAB {
                InventoryCellHit::CreativeSearch
            } else {
                InventoryCellHit::CreativeTab(*tab)
            })
        }
        // The recipe book toggle opens the panel, the survival toggle closes it.
        "layout_toggle" if (group == 2) != shown && group != 3 => {
            Some(InventoryCellHit::Widget(Widget::BookToggle))
        }
        _ => None,
    }
}
