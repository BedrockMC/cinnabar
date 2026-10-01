//! Retires immutable archives only after their replacement has been acknowledged.
use super::{CATALOG_FILE, Catalog, LibraryError, atomic_write, encode_catalog};
use std::{fs, path::Path};

/// Removes obsolete revisions after the newest preview has been acknowledged.
pub(super) fn prune(root: &Path, catalog: &mut Catalog) -> Result<(), LibraryError> {
    let mut candidate = catalog.clone();
    candidate.retained.retain(|pack| {
        candidate
            .active
            .iter()
            .any(|active| active.id == pack.id && active.revision == pack.revision)
    });
    atomic_write(&root.join(CATALOG_FILE), &encode_catalog(&candidate)?)?;
    for pack in &catalog.retained {
        if !candidate
            .retained
            .iter()
            .any(|kept| kept.id == pack.id && kept.revision == pack.revision)
        {
            let _ = fs::remove_file(root.join(pack.filename()));
        }
    }
    *catalog = candidate;
    Ok(())
}
