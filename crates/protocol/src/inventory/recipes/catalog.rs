use super::{
    budget::{Credits, MAX_RECORDS, Permit},
    model::{RecipeHandle, RecipeUpdate},
};
use std::{mem::size_of, sync::Arc};

#[derive(Debug)]
struct Entry {
    id: u32,
    handle: Option<RecipeHandle>,
}

/// Pure protocol state; this tranche does not activate an app recipe consumer.
#[derive(Debug, Default)]
pub struct RecipeCatalog {
    session: u64,
    sequence: u64,
    revision: u64,
    entries: Vec<Entry>,
    permit: Option<Permit>,
    available: bool,
    exhausted: bool,
}

impl RecipeCatalog {
    pub fn begin_session(&mut self, session: u64) {
        self.session = session;
        self.sequence = 0;
        self.retire();
    }
    fn retire(&mut self) {
        self.entries = Vec::new();
        self.permit = None;
        self.available = false;
        match self.revision.checked_add(1) {
            Some(next) => self.revision = next,
            None => self.exhausted = true,
        }
    }
    pub fn revision(&self) -> u64 {
        self.revision
    }
    pub(in crate::inventory) fn session(&self) -> u64 {
        self.session
    }
    pub fn is_available(&self) -> bool {
        self.available
    }
    pub fn recipe(&self, id: u32) -> Option<RecipeHandle> {
        let index = self
            .entries
            .binary_search_by_key(&id, |entry| entry.id)
            .ok()?;
        self.entries[index].handle.clone()
    }
    /// Every accepted FIFO update advances authority, including unavailable-only
    /// replacements. A policy refusal retires the complete previous catalog.
    pub fn apply(&mut self, session: u64, sequence: u64, update: &RecipeUpdate) -> bool {
        self.apply_with_credits(session, sequence, update, &Credits::shared())
    }
    fn apply_with_credits(
        &mut self,
        session: u64,
        sequence: u64,
        update: &RecipeUpdate,
        credits: &Arc<Credits>,
    ) -> bool {
        if self.exhausted || session != self.session || session == 0 || sequence <= self.sequence {
            return false;
        }
        self.sequence = sequence;
        if self.revision == u64::MAX {
            self.retire();
            return true;
        }
        let Some(batch) = update.batch.as_ref() else {
            self.retire();
            return true;
        };
        let old = if batch.clear {
            &[][..]
        } else {
            &self.entries[..]
        };
        let capacity = match old.len().checked_add(batch.records.len()) {
            Some(n) if n <= MAX_RECORDS * 2 => n,
            _ => {
                self.retire();
                return true;
            }
        };
        let charge = capacity
            .checked_mul(size_of::<Entry>())
            .and_then(|n| n.checked_add(128));
        let Some(permit) = charge.and_then(|n| credits.reserve(n)) else {
            self.retire();
            return true;
        };
        let mut merged = Vec::new();
        if merged.try_reserve_exact(capacity).is_err() {
            self.retire();
            return true;
        }
        let mut previous = 0;
        for (index, record) in batch.records.iter().enumerate() {
            while previous < old.len() && old[previous].id < record.id {
                merged.push(Entry {
                    id: old[previous].id,
                    handle: old[previous].handle.clone(),
                });
                previous += 1;
            }
            if previous < old.len() && old[previous].id == record.id {
                previous += 1;
            }
            merged.push(Entry {
                id: record.id,
                handle: record.recipe.as_ref().map(|_| RecipeHandle {
                    batch: Arc::clone(batch),
                    index,
                }),
            });
        }
        for entry in &old[previous..] {
            merged.push(Entry {
                id: entry.id,
                handle: entry.handle.clone(),
            });
        }
        if merged.len() > MAX_RECORDS {
            self.retire();
            return true;
        }
        self.entries = merged;
        self.permit = Some(permit);
        self.available = true;
        self.revision += 1; // checked exhaustion above, before any new authority.
        true
    }
}

#[cfg(test)]
mod tests {
    use super::super::model::{Batch, Output, Recipe, Record};
    use super::*;

    fn update(owner: &Arc<Credits>, clear: bool) -> RecipeUpdate {
        RecipeUpdate {
            batch: Some(Arc::new(Batch {
                records: vec![Record {
                    id: 17,
                    recipe: Some(Recipe {
                        width: 1,
                        height: 1,
                        ingredients: [None, None, None, None],
                        output: Output {
                            id: 7,
                            aux: 0,
                            count: 4,
                            block: 0,
                            empty_envelope: false,
                        },
                    }),
                }],
                clear,
                _permit: owner.reserve(512).unwrap(),
            })),
        }
    }

    #[test]
    fn catalog_and_external_handle_keep_batch_credit_until_final_drop() {
        let owner = Credits::isolated(4096);
        let mut catalog = RecipeCatalog::default();
        catalog.begin_session(1);
        let update = update(&owner, true);
        assert!(catalog.apply_with_credits(1, 1, &update, &owner));
        let handle = catalog.recipe(17).unwrap();
        drop(update);
        assert_eq!(owner.used(), 512 + size_of::<Entry>() + 128);
        catalog.begin_session(2);
        assert_eq!(owner.used(), 512);
        assert_eq!(handle.network_id(), 17);
        drop(handle);
        assert_eq!(owner.used(), 0);
    }

    #[test]
    fn merge_scratch_refusal_retires_catalog_without_unaccounted_storage() {
        let owner = Credits::isolated(4096);
        let mut catalog = RecipeCatalog::default();
        catalog.begin_session(1);
        let first = update(&owner, true);
        assert!(catalog.apply_with_credits(1, 1, &first, &owner));
        drop(first);
        let replacement = update(&owner, false);
        let occupied = owner.reserve(4096 - owner.used()).unwrap();
        assert!(catalog.apply_with_credits(1, 2, &replacement, &owner));
        assert!(!catalog.is_available());
        assert!(catalog.recipe(17).is_none());
        drop(replacement);
        drop(occupied);
        assert_eq!(owner.used(), 0);
    }

    #[test]
    fn oversized_merged_catalog_releases_reserved_scratch_on_policy_return() {
        let owner = Credits::isolated(2 * 1024 * 1024);
        let mut catalog = RecipeCatalog::default();
        catalog.begin_session(1);
        let first = update(&owner, true);
        catalog.apply_with_credits(1, 1, &first, &owner);
        drop(first);
        let charge = MAX_RECORDS * size_of::<Record>() + 512;
        let permit = owner.reserve(charge).unwrap();
        let replacement = RecipeUpdate {
            batch: Some(Arc::new(Batch {
                records: (100..100 + MAX_RECORDS as u32)
                    .map(|id| Record { id, recipe: None })
                    .collect(),
                clear: false,
                _permit: permit,
            })),
        };
        // Both individually bounded batches fit. Their disjoint merge reserves
        // scratch successfully, then exceeds the retained catalog cardinality.
        assert!(catalog.apply_with_credits(1, 2, &replacement, &owner));
        assert!(!catalog.is_available());
        assert_eq!(owner.used(), charge);
        drop(replacement);
        assert_eq!(owner.used(), 0);
    }
}
