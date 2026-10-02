//! Records the files a compiler actually consumes, including missing fallback candidates.

use std::{
    collections::BTreeSet,
    sync::{Arc, Mutex},
};

/// A content read or directory listing that can affect a compiled subscriber.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum PackDependency {
    File { path: String, limit: u64 },
    Directory(String),
}

/// Shared only within one compilation; clones record into the same dependency set.
#[derive(Clone, Debug, Default)]
pub struct PackDependencies(Arc<Mutex<BTreeSet<PackDependency>>>);

impl PackDependencies {
    /// Returns an immutable copy without retaining the compilation's lock.
    pub fn snapshot(&self) -> BTreeSet<PackDependency> {
        self.0
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clone()
    }

    /// Restores the inputs retained alongside a reused compilation result.
    pub fn extend(&self, inputs: impl IntoIterator<Item = PackDependency>) {
        self.0
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .extend(inputs);
    }

    /// Records an attempted read, even when the stack contains no matching file.
    pub(crate) fn file(&self, path: &str, limit: u64) {
        self.0
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .insert(PackDependency::File {
                path: path.to_owned(),
                limit,
            });
    }

    /// Directory dependencies track names and order separately from file contents.
    pub(crate) fn directory(&self, prefix: &str) {
        self.0
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .insert(PackDependency::Directory(prefix.to_owned()));
    }
}
