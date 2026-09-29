//! Bounded recipe admission. Limits are client policy, not server maxima.
mod budget;
mod catalog;
mod crafting;
mod decode;
mod grammar;
mod item_tags;
mod matching;
pub(super) mod model;
mod observation;
mod reader;
mod screen;

pub use budget::RECIPE_OWNED_BYTES;
pub use catalog::RecipeCatalog;
pub use crafting::{
    CraftGridItem, CraftGridMatch, RecipeIngredientView, RecipeOutput, match_crafting_grid,
};
pub use matching::{ManualCraftCell, ManualCraftMatch, ManualCraftPreview, match_manual_grid};
pub use model::{RecipeHandle, RecipeUpdate};
pub use screen::{
    MultiRecipe, ScreenIngredient, ScreenRecipe, ScreenRecipeKind, ScreenRecipes,
};
pub use observation::{
    IngredientObservation, MAX_RECIPE_OBSERVATIONS, RecipeObservation, RecipeObservations,
};

pub fn decode_recipe_update(body: &[u8]) -> Result<RecipeUpdate, super::InventoryPacketError> {
    decode::decode(body)
}

pub(in crate::inventory) fn valid_identifier(value: &str) -> bool {
    value.len() <= 16384 && grammar::identifier(value)
}
pub(in crate::inventory) fn empty_extra(extra: &[u8]) -> bool {
    canonical_empty_extra(extra)
}

fn canonical_empty_extra(extra: &[u8]) -> bool {
    if extra.is_empty() {
        return super::validate_item_user_data(extra).is_ok();
    }
    let mut fields = extra;
    let Some(header) = fields.get(..2) else {
        return false;
    };
    if i16::from_le_bytes([header[0], header[1]]) != 0 {
        return false;
    }
    fields = &fields[2..];
    for _ in 0..2 {
        let Some(count) = fields.get(..4) else {
            return false;
        };
        if i32::from_le_bytes([count[0], count[1], count[2], count[3]]) != 0 {
            return false;
        }
        fields = &fields[4..];
    }
    fields.is_empty() && super::validate_item_user_data(extra).is_ok()
}
