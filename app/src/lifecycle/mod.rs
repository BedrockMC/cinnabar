//! Process lifecycle around the client: crash capture, first-run asset preparation, update checks.

pub(crate) mod children;
pub(crate) mod core_health;
mod crash;
mod update;

use anyhow::{Context, Result};

use crate::{first_run, install_layout::InstallLayout};

/// Runs pre-window duties for a packaged install; a no-op for development checkouts.
/// `assets_overridden` skips asset preparation when the caller supplied its own carrier path.
pub fn before_run(assets_overridden: bool) -> Result<()> {
    let layout = InstallLayout::discover().context("resolve install layout")?;
    if !layout.is_installed() {
        return Ok(());
    }
    core_health::capture_client_stderr(&layout);
    crash::install_panic_hook(&layout);
    crash::prune_reports(&layout);
    if !assets_overridden {
        first_run::ensure_prepared(&layout)?;
    }
    update::check_in_background(&layout);
    if let Some(notice) = update::available(&layout) {
        eprintln!(
            "Cinnabar {} is available (running {}).",
            notice.latest, notice.current
        );
    }
    Ok(())
}
