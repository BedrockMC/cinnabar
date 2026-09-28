//! Precedence-ordered reads over an admitted stack.
//!
//! The last `ResourcePackStack` entry wins: the client builds its stack in list
//! order (a pack's dependencies first) and resolves a resource from the highest
//! index down. Behavior taken from the 26.30 Bedrock reconstruction; confirm
//! with a live two-pack capture.

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
            .rev()
            .find_map(|pack| pack.read_file(path).ok().flatten())
    }

    /// Like [`read`](Self::read) but reads at most `limit` uncompressed bytes; a
    /// copy over the limit is skipped, so a lower layer's copy can still win.
    #[must_use]
    pub fn read_capped(&self, path: &str, limit: u64) -> Option<Box<[u8]>> {
        self.stack
            .packs()
            .iter()
            .rev()
            .find_map(|pack| pack.read_file_with_limit(path, limit).ok().flatten())
    }

    /// Every readable copy of `path`, lowest precedence first.
    #[must_use]
    pub fn read_layers(&self, path: &str) -> Vec<Box<[u8]>> {
        self.layers()
            .filter_map(|pack| pack.read_file(path).ok().flatten())
            .collect()
    }

    /// Admitted packs, lowest precedence first.
    pub fn layers(&self) -> impl Iterator<Item = &ValidatedPack> {
        self.stack.packs().iter()
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
