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
        let Some(permit) = charge.and_then(|n| Credits::shared().reserve(n)) else {
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
