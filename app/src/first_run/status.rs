//! Machine-readable first-run progress so a launcher or UI can render it without owning the flow.

use std::{fs, path::Path};

use anyhow::{Context, Result};
use serde::Serialize;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Phase {
    AwaitingConsent,
    Running,
    Done,
    Failed,
}

#[derive(Serialize)]
pub(super) struct Status<'a> {
    pub phase: Phase,
    pub step: usize,
    pub total: usize,
    pub label: &'a str,
    pub error: Option<&'a str>,
}

/// Replaces the status file atomically so readers never observe a partial document.
pub(super) fn write(path: &Path, status: &Status<'_>) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
    }
    let temporary = path.with_extension("json.tmp");
    fs::write(&temporary, serde_json::to_vec(status)?)
        .with_context(|| format!("write {}", temporary.display()))?;
    fs::rename(&temporary, path).with_context(|| format!("publish {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::first_run::test_support::Dir;

    #[test]
    fn status_round_trips_as_snake_case_json() {
        let dir = Dir::new("status");
        let path = dir.path().join("logs/first-run-status.json");
        write(
            &path,
            &Status {
                phase: Phase::AwaitingConsent,
                step: 0,
                total: 3,
                label: "x",
                error: None,
            },
        )
        .unwrap();
        let value: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert_eq!(value["phase"], "awaiting_consent");
        assert_eq!(value["total"], 3);
        assert!(!path.with_extension("json.tmp").exists());
    }
}
