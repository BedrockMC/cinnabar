//! The screens the engine is allowed to draw, and the generic renderer for them.
//! The gameplay HUD (`hud.hud_screen` and its family) stays on the Java-styled
//! HUD path by owner decision, so it is deliberately absent from the allow-list
//! and [`render_screen`] refuses anything not on it.

use crate::bind::{DataSource, bind};
use crate::catalog::Catalog;
use crate::form::{CatalogLibrary, FormRender, finish};
use crate::layout::LayoutEnv;
use crate::state::ViewState;
use crate::{Context, resolve};

/// A rendered engine screen: bound tree, draw nodes, hit regions, scroll report.
pub type ScreenRender = FormRender;

/// Every `namespace.name` screen the engine renders. Never includes `hud_screen`.
pub const ENGINE_SCREENS: &[&str] = &[
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
    if !is_engine_screen(reference) {
        return None;
    }
    let root = resolve(catalog, reference, context).control?;
    let library = CatalogLibrary { catalog, context };
    let bound = bind(&root, data, &library);
    Some(finish(bound, root_size, env, state))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_gameplay_hud_is_never_an_engine_screen() {
        assert!(!ENGINE_SCREENS.iter().any(|screen| screen.contains("hud")));
        assert!(!is_engine_screen("hud.hud_screen"));
        assert!(is_engine_screen("chest.small_chest_screen"));
    }
}
