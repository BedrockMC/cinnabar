//! Recipe lookups for the stonecutter, smithing table and cartography table,
//! answered from the CraftingData catalog and the items in the open screen.

use std::sync::Arc;

use protocol::{RecipeCatalog, ScreenRecipe, ScreenRecipeKind, WindowKind};

use super::UiRuntime;
use super::inventory_ledger::{InventoryTarget, PlayerInventoryLedger};

/// Banner patterns the loom offers without a pattern item, as wire names
/// (provisional list, no vanilla evidence yet).
pub(crate) const LOOM_PATTERNS: [&str; 36] = [
    "bs", "ts", "ls", "rs", "cs", "ms", "drs", "dls", "ss", "cr", "sc", "ld", "rud", "lud", "rd",
    "vh", "vhr", "hh", "hhb", "bl", "br", "tl", "tr", "bt", "tt", "bts", "tts", "mc", "mr", "bo",
    "gra", "gru", "cbo", "bri", "flo", "cre",
];

struct Item {
    identifier: Arc<str>,
    metadata: u32,
    tags: Arc<[Arc<str>]>,
}

fn item_in(ledger: &PlayerInventoryLedger, slot: u8) -> Option<Item> {
    let stack = ledger.target_stack(InventoryTarget::Craft(slot))?;
    let entry = ledger.negotiated_item_entry(stack.network_id)?;
    Some(Item {
        identifier: Arc::clone(&entry.identifier),
        metadata: stack.metadata,
        tags: Arc::clone(&entry.item_tags),
    })
}

fn accepts(recipe: &ScreenRecipe, index: usize, item: &Item) -> bool {
    recipe
        .ingredients
        .get(index)
        .is_some_and(|ingredient| ingredient.accepts(&item.identifier, item.metadata, &item.tags))
}

impl UiRuntime {
    /// The committed recipe catalog while it is available.
    pub(crate) fn screen_catalog(&self) -> Option<&RecipeCatalog> {
        self.crafting_authority.catalog()
    }

    /// Stonecutter recipes the input item feeds, in catalog order.
    pub(crate) fn stonecutter_options(&self) -> Vec<&ScreenRecipe> {
        let (Some(catalog), Some(input)) =
            (self.screen_catalog(), item_in(self.inventory_ledger(), 3))
        else {
            return Vec::new();
        };
        catalog
            .screen_recipes(ScreenRecipeKind::Stonecutter)
            .filter(|recipe| accepts(recipe, 0, &input))
            .collect()
    }

    /// The recipe the open stonecutter, smithing or cartography screen applies now.
    pub(crate) fn active_screen_recipe(&self) -> Option<&ScreenRecipe> {
        let ledger = self.inventory_ledger();
        let catalog = self.screen_catalog()?;
        match ledger.window_kind()? {
            WindowKind::Stonecutter => {
                let choice = self.screen_state().recipe_choice?;
                self.stonecutter_options()
                    .into_iter()
                    .find(|recipe| recipe.id == choice)
            }
            WindowKind::Smithing => {
                let (template, base, addition) = (
                    item_in(ledger, 53)?,
                    item_in(ledger, 51)?,
                    item_in(ledger, 52)?,
                );
                catalog
                    .screen_recipes(ScreenRecipeKind::SmithingTransform)
                    .find(|recipe| {
                        accepts(recipe, 0, &template)
                            && accepts(recipe, 1, &base)
                            && accepts(recipe, 2, &addition)
                    })
            }
            WindowKind::Cartography => {
                let (map, extra) = (item_in(ledger, 12)?, item_in(ledger, 13)?);
                catalog
                    .screen_recipes(ScreenRecipeKind::Cartography)
                    .find(|recipe| {
                        (accepts(recipe, 0, &map) && accepts(recipe, 1, &extra))
                            || (accepts(recipe, 0, &extra) && accepts(recipe, 1, &map))
                    })
            }
            _ => None,
        }
    }
}
