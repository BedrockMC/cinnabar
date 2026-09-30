//! First-run preparation of the Mojang-derived asset carriers, which installers never ship.
//!
//! Runs before the window opens: consent, then fetch of the pinned public pack, then `assetc`.
//! Progress is mirrored to `logs/first-run-status.json`.

mod plan;
mod runner;
mod status;
#[cfg(test)]
mod test_support;

use std::fs::{self, OpenOptions};

use anyhow::{Context, Result, bail};

use crate::{
    install_layout::InstallLayout,
    native_dialog::{NativePrompter, Prompter},
};
use status::{Phase, Status};

const CONSENT_ENV: &str = "CINNABAR_ACCEPT_MOJANG_EULA";
const TITLE: &str = "Cinnabar first-time setup";
const CONSENT_BODY: &str = "Cinnabar needs Minecraft's official sample resource pack. It is downloaded from Mojang's public release (a large one-time download), converted on this computer, and never redistributed by Cinnabar.\n\nContinuing confirms you accept the Minecraft EULA (https://www.minecraft.net/eula). Setup runs once and takes a few minutes.";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Outcome {
    NotNeeded,
    Prepared,
}

/// Prepares the per-user carriers when a packaged install has none; a no-op for development checkouts.
pub(crate) fn ensure_prepared(layout: &InstallLayout) -> Result<Outcome> {
    ensure_with(
        layout,
        &NativePrompter,
        std::env::var_os(CONSENT_ENV).is_some_and(|v| v == "1"),
    )
}

fn ensure_with(
    layout: &InstallLayout,
    prompter: &dyn Prompter,
    env_consent: bool,
) -> Result<Outcome> {
    if !layout.is_installed() || plan::carriers_present(&layout.compiled_assets) {
        return Ok(Outcome::NotNeeded);
    }
    let status_path = layout.log_dir().join("first-run-status.json");
    let report = |phase, step, total, label: &str, error: Option<&str>| {
        let _ = status::write(
            &status_path,
            &Status {
                phase,
                step,
                total,
                label,
                error,
            },
        );
    };
    report(Phase::AwaitingConsent, 0, 0, "Waiting for consent", None);
    let marker = layout.prepare_workspace().join("eula-accepted");
    if !env_consent && !marker.is_file() && !prompter.confirm(TITLE, CONSENT_BODY) {
        report(Phase::Failed, 0, 0, "Declined", Some("setup declined"));
        bail!("first-time setup was declined; Cinnabar cannot start without its game assets");
    }
    if let Some(parent) = marker.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&marker, b"accepted\n").context("record consent")?;
    prompter.info(
        TITLE,
        "Preparing game assets. This happens once and takes a few minutes.",
    );
    match prepare(layout, |step, total, label| {
        report(Phase::Running, step, total, label, None)
    }) {
        Ok(()) => {
            report(Phase::Done, 0, 0, "Ready", None);
            prompter.info(TITLE, "Setup finished. Starting Cinnabar.");
            Ok(Outcome::Prepared)
        }
        Err(error) => {
            let message = format!("{error:#}");
            report(Phase::Failed, 0, 0, "Failed", Some(message.as_str()));
            prompter.alert(
                TITLE,
                &format!(
                    "Setup failed: {message}\n\nDetails: {}",
                    layout.log_dir().join("first-run.log").display()
                ),
            );
            Err(error)
        }
    }
}

fn prepare(layout: &InstallLayout, mut progress: impl FnMut(usize, usize, &str)) -> Result<()> {
    let kit = layout.prep_kit();
    if !kit.is_dir() {
        bail!(
            "installer preparation kit is missing at {}; reinstall Cinnabar",
            kit.display()
        );
    }
    let workspace = layout.prepare_workspace();
    runner::stage_kit(&kit, &workspace)?;
    let steps = plan::steps(&workspace)?;
    let staged = workspace.join(".local/assets/compiled");
    if staged.exists() {
        fs::remove_dir_all(&staged)?;
    }
    fs::create_dir_all(&staged)?;
    fs::create_dir_all(layout.log_dir())?;
    let log_path = layout.log_dir().join("first-run.log");
    let log = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)
        .with_context(|| format!("open {}", log_path.display()))?;
    let exec = runner::ProcessExec {
        workspace: workspace.clone(),
        kit,
        log,
    };
    let total = steps.len();
    runner::execute_steps(
        &steps,
        |step| exec.run(step),
        |index, step| progress(index + 1, total, step.label),
    )?;
    if !plan::carriers_present(&staged) {
        bail!(
            "preparation finished but required carriers are missing under {}",
            staged.display()
        );
    }
    runner::publish(&staged, &layout.prepared_assets_dir())?;
    // The extracted pack and its archive are only compile inputs; carriers are all that persist.
    for scratch in ["bedrock-samples", "downloads"] {
        let _ = fs::remove_dir_all(workspace.join(".local/assets").join(scratch));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::{cell::Cell, path::PathBuf};

    use super::*;
    use crate::install_layout::{InstallEnvironment, Platform};
    use test_support::Dir;

    struct Fake {
        accept: bool,
        asked: Cell<u32>,
    }

    impl Prompter for Fake {
        fn confirm(&self, _: &str, _: &str) -> bool {
            self.asked.set(self.asked.get() + 1);
            self.accept
        }
        fn info(&self, _: &str, _: &str) {}
        fn alert(&self, _: &str, _: &str) {}
    }

    fn installed_layout(data: &Dir, executable: &str) -> InstallLayout {
        InstallLayout::resolve(
            Platform::Linux,
            &InstallEnvironment {
                executable: PathBuf::from(executable),
                home: Some(PathBuf::from("/home/dev")),
                local_app_data: None,
                xdg_config_home: Some(data.path().join("cfg")),
                xdg_data_home: Some(data.path().join("data")),
                xdg_runtime_dir: None,
            },
        )
        .unwrap()
        .with_prepared_assets()
    }

    #[test]
    fn development_layout_needs_no_preparation() {
        let data = Dir::new("dev");
        let layout = installed_layout(&data, "/work/cinnabar/target/release/bedrock-client");
        let fake = Fake {
            accept: false,
            asked: Cell::new(0),
        };
        assert_eq!(
            ensure_with(&layout, &fake, false).unwrap(),
            Outcome::NotNeeded
        );
        assert_eq!(fake.asked.get(), 0);
    }

    #[test]
    fn declined_consent_fails_closed_without_running_anything() {
        let data = Dir::new("decline");
        let layout = installed_layout(&data, "/nonexistent/opt/cinnabar/bin/bedrock-client");
        let fake = Fake {
            accept: false,
            asked: Cell::new(0),
        };
        let error = ensure_with(&layout, &fake, false).unwrap_err();
        assert!(error.to_string().contains("declined"));
        assert!(!layout.prepare_workspace().join("eula-accepted").exists());
    }

    #[test]
    fn missing_kit_is_reported_after_consent_and_recorded() {
        let data = Dir::new("nokit");
        let layout = installed_layout(&data, "/nonexistent/opt/cinnabar/bin/bedrock-client");
        let fake = Fake {
            accept: true,
            asked: Cell::new(0),
        };
        let error = ensure_with(&layout, &fake, false).unwrap_err();
        assert!(format!("{error:#}").contains("preparation kit"));
        // Consent is remembered, so the retry does not prompt again.
        let _ = ensure_with(&layout, &fake, false);
        assert_eq!(fake.asked.get(), 1);
    }
}
