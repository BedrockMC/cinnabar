//! The screens the engine is allowed to draw, and the generic renderer for them;
//! [`render_screen`] refuses anything not on the allow-list.

use crate::bind::{DataSource, bind};
use crate::catalog::Catalog;
use crate::form::{CatalogLibrary, FormRender, finish};
use crate::layout::LayoutEnv;
use crate::state::ViewState;
use crate::{Context, ResolvedControl, resolve};

/// A rendered engine screen: bound tree, draw nodes, hit regions, scroll report.
pub type ScreenRender = FormRender;

/// Every `namespace.name` screen the engine renders.
pub const ENGINE_SCREENS: &[&str] = &[
    crate::hud::HUD_SCREEN,
    crate::hud::CROSSHAIR_SCREEN,
    "server_form.third_party_server_screen",
    "server_form.long_form",
    "server_form.custom_form",
    "popup_dialog.modal_dialog_popup",
    "crafting.inventory_screen",
    "crafting.crafting_screen",
    "chest.small_chest_screen",
    "chest.large_chest_screen",
    "chest.ender_chest_screen",
    "chest.barrel_screen",
    "chest.shulker_box_screen",
    "furnace.furnace_screen",
    "blast_furnace.blast_furnace_screen",
    "smoker.smoker_screen",
    "anvil.anvil_screen",
    "enchanting.enchanting_screen",
    "brewing_stand.brewing_stand_screen",
    "grindstone.grindstone_screen",
    "loom.loom_screen",
    "smithing_table.smithing_table_screen",
    "cartography.cartography_screen",
    "stonecutter.stonecutter_screen",
    "beacon.beacon_screen",
    "redstone.hopper_screen",
    "redstone.dispenser_screen",
    "redstone.dropper_screen",
    "redstone.crafter_screen",
    "horse.horse_screen",
    "book.book_screen",
    "npc_interact.npc_screen",
    "pause.pause_screen",
    "start.start_screen",
    "play.play_screen",
    "add_external_server.add_external_server_screen_new",
    "settings.screen_controls_and_settings",
    "death.death_screen",
    "progress.progress_screen",
    "disconnect.disconnect_screen",
    "xbl_console_signin.xbl_console_signin",
    "store_layout.store_data_driven_screen",
    "store_inventory.store_inventory_screen",
    "store_progress.store_progress_screen",
];

pub fn is_engine_screen(reference: &str) -> bool {
    ENGINE_SCREENS.contains(&reference)
}

/// Resolve, bind, lay out, and emit an allow-listed screen against `data`.
/// `None` for a screen off the allow-list or a reference the catalog lacks.
pub fn render_screen(
    reference: &str,
    catalog: &Catalog,
    context: &Context,
    data: &DataSource,
    root_size: [f64; 2],
    env: &LayoutEnv,
    state: &ViewState,
) -> Option<ScreenRender> {
    let root = resolve_screen(reference, catalog, context)?;
    let bound = bind_screen(&root, catalog, context, data);
    Some(finish(bound, root_size, env, state))
}

/// Resolve an allow-listed screen; `None` off the allow-list or when the
/// catalog lacks it. The result depends only on its inputs, so callers may
/// keep it while those stay the same.
pub fn resolve_screen(
    reference: &str,
    catalog: &Catalog,
    context: &Context,
) -> Option<ResolvedControl> {
    if !is_engine_screen(reference) {
        return None;
    }
    resolve(catalog, reference, context).control
}

/// Bind a resolved screen against `data`, ready for [`crate::render_bound`].
pub fn bind_screen(
    root: &ResolvedControl,
    catalog: &Catalog,
    context: &Context,
    data: &DataSource,
) -> ResolvedControl {
    bind(root, data, &CatalogLibrary { catalog, context })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_gameplay_hud_is_an_engine_screen() {
        assert!(is_engine_screen("hud.hud_screen"));
        assert!(is_engine_screen("hud_crosshair.hud_crosshair_screen"));
        assert!(!is_engine_screen("hud.hud_content"));
    }
}
