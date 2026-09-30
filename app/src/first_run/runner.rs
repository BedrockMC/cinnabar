//! Executes the preparation plan as child processes and publishes the finished carriers.

use std::{
    fs::{self, File},
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

use anyhow::{Context, Result, bail};

use super::plan::{Action, Step};

/// Runs `steps` in order; a failed required step aborts, a failed optional step is returned as skipped.
pub(super) fn execute_steps(
    steps: &[Step],
    mut exec: impl FnMut(&Step) -> Result<()>,
    mut progress: impl FnMut(usize, &Step),
) -> Result<Vec<&'static str>> {
    let mut skipped = Vec::new();
    for (index, step) in steps.iter().enumerate() {
        progress(index, step);
        if let Err(error) = exec(step) {
            if step.required {
                return Err(error.context(step.label));
            }
            skipped.push(step.label);
        }
    }
    Ok(skipped)
}

/// Copies the bundled scripts, manifests and registries into the workspace at repo-relative paths.
pub(super) fn stage_kit(kit: &Path, workspace: &Path) -> Result<()> {
    for (from, to) in [
        ("scripts", "scripts"),
        ("assets", "assets"),
        ("data", "crates/assets/data"),
    ] {
        copy_tree(&kit.join(from), &workspace.join(to))?;
    }
    Ok(())
}

fn copy_tree(from: &Path, to: &Path) -> Result<()> {
    fs::create_dir_all(to).with_context(|| format!("create {}", to.display()))?;
    for entry in fs::read_dir(from).with_context(|| format!("read {}", from.display()))? {
        let entry = entry?;
        let target = to.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_tree(&entry.path(), &target)?;
        } else {
            fs::copy(entry.path(), &target)
                .with_context(|| format!("copy {}", entry.path().display()))?;
        }
    }
    Ok(())
}

/// Moves the fully built carrier directory into place, replacing any earlier one.
pub(super) fn publish(staged: &Path, final_dir: &Path) -> Result<()> {
    if let Some(parent) = final_dir.parent() {
        fs::create_dir_all(parent)?;
    }
    if final_dir.exists() {
        fs::remove_dir_all(final_dir).with_context(|| format!("remove {}", final_dir.display()))?;
    }
    fs::rename(staged, final_dir)
        .with_context(|| format!("move {} to {}", staged.display(), final_dir.display()))
}

pub(super) struct ProcessExec {
    pub workspace: PathBuf,
    pub kit: PathBuf,
    pub log: File,
}

impl ProcessExec {
    pub(super) fn run(&self, step: &Step) -> Result<()> {
        let mut command = self.command(&step.action)?;
        command
            .current_dir(&self.workspace)
            .stdin(Stdio::null())
            .stdout(self.log.try_clone()?)
            .stderr(self.log.try_clone()?);
        let status = command
            .status()
            .with_context(|| format!("start {}", step.label))?;
        if !status.success() {
            bail!("{} exited with {status}; see the first-run log", step.label);
        }
        Ok(())
    }

    fn command(&self, action: &Action) -> Result<Command> {
        match action {
            Action::Assetc(args) => {
                let mut command = Command::new(self.kit.join("bin").join(assetc_name()));
                command.args(args);
                Ok(command)
            }
            Action::Script(name) => Ok(script_command(&self.kit, name)),
        }
    }
}

const fn assetc_name() -> &'static str {
    if cfg!(windows) {
        "assetc.exe"
    } else {
        "assetc"
    }
}

fn script_command(kit: &Path, name: &str) -> Command {
    if cfg!(windows) {
        let mut command = Command::new("powershell");
        command
            .args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-File"])
            .arg(format!("scripts\\{name}.ps1"));
        if name == "fetch-vanilla-assets" {
            command.arg("-AcceptEula");
        }
        command
    } else {
        let mut command = Command::new("bash");
        command.arg(format!("scripts/{name}.sh"));
        if name == "fetch-vanilla-assets" {
            command.arg("--accept-eula");
            let helper = kit.join("bin/rename-directory-no-replace");
            if helper.is_file() {
                command.env("CINNABAR_PUBLISHER_BINARY", helper);
            }
        }
        command
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::first_run::test_support::Dir;

    fn step(label: &'static str, required: bool) -> Step {
        Step {
            label,
            action: Action::Script("x"),
            required,
        }
    }

    #[test]
    fn required_failure_aborts_and_optional_failure_is_skipped() {
        let steps = [step("a", true), step("b", false), step("c", true)];
        let skipped = execute_steps(
            &steps,
            |s| if s.label == "b" { bail!("no") } else { Ok(()) },
            |_, _| {},
        )
        .unwrap();
        assert_eq!(skipped, ["b"]);

        let mut ran = Vec::new();
        let error = execute_steps(
            &steps,
            |s| {
                ran.push(s.label);
                if s.label == "a" {
                    bail!("boom")
                } else {
                    Ok(())
                }
            },
            |_, _| {},
        )
        .unwrap_err();
        assert_eq!(ran, ["a"]);
        assert!(format!("{error:#}").contains("boom"));
    }

    #[test]
    fn progress_reports_each_step_index_in_order() {
        let steps = [step("a", true), step("b", true)];
        let mut seen = Vec::new();
        execute_steps(&steps, |_| Ok(()), |index, s| seen.push((index, s.label))).unwrap();
        assert_eq!(seen, [(0, "a"), (1, "b")]);
    }

    #[test]
    fn stage_kit_maps_data_under_the_registry_path() {
        let dir = Dir::new("kit");
        let kit = dir.path().join("kit");
        for sub in ["scripts", "assets", "data"] {
            fs::create_dir_all(kit.join(sub)).unwrap();
            fs::write(kit.join(sub).join("f"), sub).unwrap();
        }
        let workspace = dir.path().join("ws");
        stage_kit(&kit, &workspace).unwrap();
        assert_eq!(
            fs::read(workspace.join("crates/assets/data/f")).unwrap(),
            b"data"
        );
        assert!(workspace.join("scripts/f").is_file());
        assert!(workspace.join("assets/f").is_file());
    }

    #[test]
    fn publish_replaces_an_earlier_directory() {
        let dir = Dir::new("publish");
        let (staged, final_dir) = (dir.path().join("staged"), dir.path().join("out/final"));
        fs::create_dir_all(&staged).unwrap();
        fs::write(staged.join("new"), b"1").unwrap();
        fs::create_dir_all(&final_dir).unwrap();
        fs::write(final_dir.join("old"), b"1").unwrap();
        publish(&staged, &final_dir).unwrap();
        assert!(final_dir.join("new").is_file() && !final_dir.join("old").exists());
    }
}
