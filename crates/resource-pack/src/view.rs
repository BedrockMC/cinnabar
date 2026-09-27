//! Precedence-ordered reads over an admitted stack.
//!
//! The first `ResourcePackStack` entry has the highest precedence: servers list
//! built-in base packs last, and the client applies stack layers from the last
//! entry to the first so earlier entries override later ones.

use std::{collections::BTreeSet, sync::Arc};

use crate::{ValidatedPack, ValidatedPackStack};

/// A read-only merged namespace over one session's admitted packs.
#[derive(Clone, Debug)]
pub struct LayeredPackView {
    stack: Arc<ValidatedPackStack>,
}

impl LayeredPackView {
    #[must_use]
    pub const fn new(stack: Arc<ValidatedPackStack>) -> Self {
        Self { stack }
    }

    #[must_use]
    pub fn stack(&self) -> &ValidatedPackStack {
        &self.stack
    }

    /// Returns the winning copy of `path`. A pack whose copy cannot be read is
    /// skipped so the next layer down can still supply it.
    #[must_use]
    pub fn read(&self, path: &str) -> Option<Box<[u8]>> {
        self.stack
            .packs()
            .iter()
            .find_map(|pack| pack.read_file(path).ok().flatten())
    }

    /// Returns every readable copy of `path`, lowest precedence first, so a
    /// merge that lets later entries override earlier ones matches the stack.
    #[must_use]
    pub fn read_layers(&self, path: &str) -> Vec<Box<[u8]>> {
        self.layers()
            .filter_map(|pack| pack.read_file(path).ok().flatten())
            .collect()
    }

    /// Admitted packs, lowest precedence first.
    pub fn layers(&self) -> impl Iterator<Item = &ValidatedPack> {
        self.stack.packs().iter().rev()
    }

    /// Lists the union of logical files under `prefix` in lexical order.
    #[must_use]
    pub fn list(&self, prefix: &str) -> Vec<&str> {
        let mut paths = BTreeSet::new();
        for pack in self.stack.packs() {
            paths.extend(pack.files_under(prefix).iter().copied());
        }
        paths.into_iter().collect()
    }
}
