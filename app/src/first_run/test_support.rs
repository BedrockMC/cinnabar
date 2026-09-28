use std::path::{Path, PathBuf};

/// Self-deleting scratch directory unique per process and label.
pub(super) struct Dir(PathBuf);

impl Dir {
    pub(super) fn new(label: &str) -> Self {
        let path =
            std::env::temp_dir().join(format!("cinnabar-first-run-{}-{label}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }

    pub(super) fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for Dir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
