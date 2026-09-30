use std::{
    path::Path,
    process::{Child, Command, Stdio},
};

use anyhow::{Context, Result};

use crate::{install_layout::InstallLayout, menu::core_process::clear_stale_bridge_endpoint};

/// Spawns a core that serves the world-control methods and routes the game socket to local worlds.
pub(crate) fn spawn_core_for_local_worlds(
    layout: &InstallLayout,
    socket_dir: &Path,
) -> Result<Child> {
    let executable = &layout.core_executable;
    if !executable.is_file() {
        anyhow::bail!(
            "bedrock-core executable was not found at {}",
            executable.display()
        );
    }
    clear_stale_bridge_endpoint(socket_dir)?;
    local_worlds_command(layout, socket_dir)
        .spawn()
        .with_context(|| format!("spawn {} for local worlds", executable.display()))
}

pub(super) fn local_worlds_command(layout: &InstallLayout, socket_dir: &Path) -> Command {
    let mut command = Command::new(&layout.core_executable);
    command
        .arg("-socket-dir")
        .arg(socket_dir)
        .arg("-control-status")
        .arg("-local-worlds-dir")
        .arg(layout.local_worlds_dir())
        .arg("-resource-pack-cache-dir")
        .arg(layout.resource_pack_cache_dir())
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    command
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use crate::install_layout::{InstallEnvironment, Platform};

    use super::*;

    fn layout() -> InstallLayout {
        InstallLayout::resolve(
            Platform::Linux,
            &InstallEnvironment {
                executable: PathBuf::from("/opt/cinnabar/bin/bedrock-client"),
                home: Some(PathBuf::from("/home/p")),
                local_app_data: None,
                xdg_config_home: None,
                xdg_data_home: Some(PathBuf::from("/data")),
                xdg_runtime_dir: Some(PathBuf::from("/run/user/1")),
            },
        )
        .expect("layout")
    }

    #[test]
    fn command_enables_control_and_local_worlds_without_an_upstream() {
        let layout = layout();
        let command = local_worlds_command(&layout, Path::new("/run/s"));
        let args: Vec<String> = command
            .get_args()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect();
        assert!(args.iter().any(|arg| arg == "-control-status"));
        assert!(!args.iter().any(|arg| arg == "-upstream"));
        let position = args
            .iter()
            .position(|arg| arg == "-local-worlds-dir")
            .expect("worlds dir flag");
        assert_eq!(
            Path::new(&args[position + 1]),
            layout.local_worlds_dir().as_path()
        );
    }
}
