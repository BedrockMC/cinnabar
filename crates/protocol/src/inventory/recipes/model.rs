use super::budget::Permit;
use std::sync::Arc;

/// Grid cells a crafting-table recipe may address.
pub(in crate::inventory) const MAX_INGREDIENTS: usize = 9;
/// Ingredient metadata that accepts any item metadata.
pub(in crate::inventory) const ANY_AUX: u16 = 32767;

#[derive(Debug, PartialEq, Eq)]
pub(in crate::inventory) struct Ingredient {
    /// An item identifier, or a tag when `tag` is set.
    pub(in crate::inventory) name: String,
    pub(in crate::inventory) tag: bool,
    pub(in crate::inventory) aux: u16,
    pub(in crate::inventory) count: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::inventory) struct Output {
    pub(in crate::inventory) id: i32,
    pub(in crate::inventory) aux: u16,
    pub(in crate::inventory) count: u8,
    pub(in crate::inventory) block: u32,
    pub(in crate::inventory) empty_envelope: bool,
}

#[derive(Debug, PartialEq, Eq)]
pub(in crate::inventory) struct Recipe {
    /// Zero for a shapeless recipe.
    pub(in crate::inventory) width: u8,
    pub(in crate::inventory) height: u8,
    pub(in crate::inventory) shapeless: bool,
    /// A shaped recipe that also matches its horizontal mirror.
    pub(in crate::inventory) mirror: bool,
    /// Lower values win when several recipes match one grid.
    pub(in crate::inventory) priority: i32,
    /// Row-major shaped cells, or the shapeless ingredient list.
    pub(in crate::inventory) ingredients: [Option<Ingredient>; MAX_INGREDIENTS],
    pub(in crate::inventory) output: Output,
}

impl Ingredient {
    pub(in crate::inventory) fn accepts_metadata(&self, metadata: u32) -> bool {
        self.aux == ANY_AUX || u32::from(self.aux) == metadata
    }
}

impl Recipe {
    /// The shaped, name-only, fits-in-two-by-two domain the manual craft
    /// builder and passive observations were written for.
    pub(in crate::inventory) fn is_personal_named(&self) -> bool {
        !self.shapeless
            && self.width <= 2
            && self.height <= 2
            && self
                .ingredients
                .iter()
                .flatten()
                .all(|ingredient| !ingredient.tag)
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(in crate::inventory) struct Record {
    pub(in crate::inventory) id: u32,
    pub(in crate::inventory) recipe: Option<Recipe>,
}

#[derive(Debug)]
pub(in crate::inventory) struct Batch {
    pub(in crate::inventory) records: Vec<Record>,
    pub(in crate::inventory) clear: bool,
    pub(super) _permit: Permit,
}

/// Cloning shares the immutable allocation and its lifetime credit.
#[derive(Debug, Clone)]
pub struct RecipeUpdate {
    pub(in crate::inventory) batch: Option<Arc<Batch>>,
    pub(in crate::inventory) screen: Option<Arc<super::screen::ScreenRecipes>>,
}

impl RecipeUpdate {
    pub fn is_unavailable(&self) -> bool {
        self.batch.is_none()
    }
    pub fn clears_catalog(&self) -> bool {
        self.batch.as_ref().is_none_or(|b| b.clear)
    }
    pub fn record_count(&self) -> usize {
        self.batch.as_ref().map_or(0, |b| b.records.len())
    }
    pub(in crate::inventory) fn unavailable() -> Self {
        Self {
            batch: None,
            screen: None,
        }
    }
}

impl PartialEq for RecipeUpdate {
    fn eq(&self, other: &Self) -> bool {
        self.screen == other.screen
            && match (&self.batch, &other.batch) {
                (Some(a), Some(b)) => a.clear == b.clear && a.records == b.records,
                (None, None) => true,
                _ => false,
            }
    }
}
impl Eq for RecipeUpdate {}

/// A handle cannot detach or clone owned recipe storage from its charged batch.
#[derive(Debug, Clone)]
pub struct RecipeHandle {
    pub(in crate::inventory) batch: Arc<Batch>,
    pub(in crate::inventory) index: usize,
}
impl RecipeHandle {
    pub fn network_id(&self) -> u32 {
        self.batch.records[self.index].id
    }
    pub fn dimensions(&self) -> (u8, u8) {
        let recipe = self.recipe();
        (recipe.width, recipe.height)
    }
    pub(in crate::inventory) fn recipe(&self) -> &Recipe {
        self.batch.records[self.index]
            .recipe
            .as_ref()
            .expect("supported handle invariant")
    }
}
