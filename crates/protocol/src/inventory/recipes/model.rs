use super::budget::Permit;
use std::sync::Arc;

#[derive(Debug, PartialEq, Eq)]
pub(in crate::inventory) struct Ingredient {
    pub(in crate::inventory) name: String,
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
    pub(in crate::inventory) width: u8,
    pub(in crate::inventory) height: u8,
    pub(in crate::inventory) ingredients: [Option<Ingredient>; 4],
    pub(in crate::inventory) output: Output,
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
        Self { batch: None }
    }
}

impl PartialEq for RecipeUpdate {
    fn eq(&self, other: &Self) -> bool {
        match (&self.batch, &other.batch) {
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
