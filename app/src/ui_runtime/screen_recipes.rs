//! Recipe lookups for the stonecutter, smithing table and cartography table,
//! answered from the CraftingData catalog and the items in the open screen.

use std::sync::Arc;

use protocol::{RecipeCatalog, RecipeHandle, ScreenRecipe, ScreenRecipeKind, WindowKind};

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

    /// The result the open screen previews before it is taken: the chosen
    /// recipe's on the stonecutter, smithing and cartography tables, the
    /// grid's recipe on a crafter.
    pub(crate) fn predicted_screen_output(&self) -> Option<protocol::RecipeOutput> {
        match self.inventory_ledger().window_kind()? {
            WindowKind::Stonecutter | WindowKind::Smithing | WindowKind::Cartography => {
                self.active_screen_recipe()?.output
            }
            WindowKind::Crafter => {
                let cells = self.inventory_ledger().crafter_grid_cells()?;
                if cells.iter().all(Option::is_none) {
                    return None;
                }
                let items: Vec<_> = cells
                    .iter()
                    .map(|cell| {
                        cell.as_ref()
                            .map(super::inventory_ledger::CraftGridCell::item)
                    })
                    .collect();
                match protocol::match_crafting_grid(self.screen_catalog()?, 3, &items) {
                    protocol::CraftGridMatch::Unique(recipe) => Some(recipe.output()),
                    _ => None,
                }
            }
            _ => None,
        }
    }

    /// Whether the recipe book filters by the inventory: on by default outside
    /// creative, as `CraftingScreenController` opens.
    pub(crate) fn recipe_filtering(&self) -> bool {
        self.screen_state()
            .recipe_filtering
            .unwrap_or(self.player_game_mode() != Some(protocol::PlayerGameMode::Creative))
    }

    /// Crafting recipes the recipe book lists for the open grid, after
    /// skipping `skip`, at most `take`. Filtering keeps the craftable ones,
    /// first, then those the inventory holds some ingredient of; otherwise
    /// every recipe lists, the uncraftable ones shown disabled.
    pub(crate) fn book_recipes(&self, skip: usize, take: usize) -> Vec<RecipeHandle> {
        let Some(catalog) = self.screen_catalog() else {
            return Vec::new();
        };
        let ledger = self.inventory_ledger();
        let small = ledger.window_kind() != Some(WindowKind::Workbench);
        let filtering = self.recipe_filtering();
        let mut listed: Vec<(bool, RecipeHandle)> = catalog
            .crafting_handles()
            .into_iter()
            .filter(|recipe| {
                let (width, height) = recipe.dimensions();
                !small
                    || if recipe.is_shapeless() {
                        recipe.ingredient_views().len() <= 4
                    } else {
                        width <= 2 && height <= 2
                    }
            })
            .map(|recipe| (ledger.can_auto_craft(&recipe), recipe))
            .filter(|(craftable, recipe)| {
                !filtering || *craftable || ledger.holds_any_ingredient(recipe)
            })
            .collect();
        if filtering {
            listed.sort_by_key(|(craftable, _)| !craftable);
        }
        listed
            .into_iter()
            .map(|(_, recipe)| recipe)
            .skip(skip)
            .take(take)
            .collect()
    }
}
