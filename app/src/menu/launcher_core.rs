//! The launcher's long-lived core: one `-control-status` core serves account,
//! catalog, connect and local-world control for the whole launcher run. Joins
//! pick their target over `connect.v1` and dial this core's game socket, so it
//! restarts only when the validated sign-in changes (sign-in or sign-out).

use std::{
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::Duration,
};

use anyhow::{Context, Result, anyhow};
use bevy::prelude::{Commands, Resource};
use protocol::launcher_control::{self, ConnectTarget};

use super::{
    AuthState, AuthSupervisor, CoreProcessGuard, MenuRuntime, account,
    core_process::{clear_stale_bridge_endpoint, core_executable},
    launcher_account::LauncherAccount,
    wait_for_core,
};
use crate::{
    install_layout::InstallLayout, local_worlds::LocalWorlds,
    runtime::endpoint::bridge_endpoint_exists, session_cleanup::SessionDirectoryGuard,
};

/// Session generation reserved for the launcher core's directory; sessions start at 1.
const LAUNCHER_GENERATION: u64 = 0;
/// How long a join waits for the core to answer `connect.v1`.
const SELECT_TIMEOUT: Duration = Duration::from_secs(3);
const DEFAULT_PORT: u16 = 19132;
const LOCAL_SERVER: &str = if cfg!(windows) {
    "bedrock-local-server.exe"
} else {
    "bedrock-local-server"
};

/// Present only in launcher runs; holds the core once started.
#[derive(Default, Resource)]
pub(crate) struct LauncherCoreSlot {
    core: Option<LauncherCore>,
    /// Sign-in mode whose spawn failed; retried only once the mode changes.
    failed: Option<bool>,
}

struct LauncherCore {
    _guard: CoreProcessGuard, // declared first: the core stops before its directory goes
    _directory: SessionDirectoryGuard,
    socket_dir: PathBuf,
    authenticated: bool,
    /// Account and local-world clients are attached once the game socket is up.
    attached: bool,
}

impl LauncherCoreSlot {
    /// Keep the core matching the validated sign-in while idle, and attach the
    /// account and local-world clients once it is serving.
    pub(super) fn drive(
        &mut self,
        commands: &mut Commands,
        menu: &mut MenuRuntime,
        idle: bool,
        upstream_client_cache: bool,
        mut worlds: Option<&mut LocalWorlds>,
    ) {
        if idle && !menu.sign_in_in_flight() {
            let auth_cache = menu.launcher_auth_cache();
            let wanted = auth_cache.is_some();
            let current = self.core.as_ref().map(|core| core.authenticated);
            if current != Some(wanted) && self.failed != Some(wanted) {
                if let Some(old) = self.core.take() {
                    drop(old);
                    commands.remove_resource::<LauncherAccount>();
                    if let Some(worlds) = worlds.as_deref_mut() {
                        worlds.detach();
                    }
                    menu.control_auth = None;
                }
                match LauncherCore::spawn(
                    &menu.layout,
                    auth_cache.as_deref(),
                    upstream_client_cache,
                ) {
                    Ok(core) => {
                        self.core = Some(core);
                        self.failed = None;
                    }
                    Err(error) => {
                        bevy::log::warn!("launcher core unavailable: {error:#}");
                        self.failed = Some(wanted);
                    }
                }
            }
        }
        if let Some(core) = self.core.as_mut()
            && !core.attached
            && bridge_endpoint_exists(&core.socket_dir)
        {
            core.attached = true;
            commands.insert_resource(LauncherAccount::new(core.socket_dir.clone()));
            if let Some(worlds) = worlds
                && let Err(error) = worlds.attach(core.socket_dir.clone())
            {
                bevy::log::warn!("local worlds unavailable: {error}");
            }
        }
    }

    /// The game socket a join dials after selecting its target on the launcher
    /// core; `None` leaves the join to a per-session core.
    pub(super) fn prepare_join(
        &self,
        address: &str,
        local_world: bool,
        authenticated: bool,
    ) -> Option<Result<PathBuf, String>> {
        let core = self.core.as_ref()?;
        if !local_world && core.authenticated != authenticated {
            return None;
        }
        Some(core.prepare(address, local_world))
    }
}

impl LauncherCore {
    fn spawn(
        layout: &InstallLayout,
        auth_cache: Option<&Path>,
        upstream_client_cache: bool,
    ) -> Result<Self> {
        let executable =
            core_executable(layout).ok_or_else(|| anyhow!("bedrock-core executable not found"))?;
        let socket_dir = layout.connect_socket_dir(std::process::id(), LAUNCHER_GENERATION);
        let directory =
            SessionDirectoryGuard::bind(socket_dir.clone()).map_err(|error| anyhow!("{error}"))?;
        clear_stale_bridge_endpoint(&socket_dir)?;
        let child = crate::lifecycle::children::spawn(&mut launcher_command(
            layout,
            &executable,
            &socket_dir,
            auth_cache,
            upstream_client_cache,
        ))
        .with_context(|| format!("spawn {} for the launcher", executable.display()))?;
        let mut guard = CoreProcessGuard::default();
        guard.replace(child);
        Ok(Self {
            _guard: guard,
            _directory: directory,
            socket_dir,
            authenticated: auth_cache.is_some(),
            attached: false,
        })
    }

    fn prepare(&self, address: &str, local_world: bool) -> Result<PathBuf, String> {
        wait_for_core(&self.socket_dir).map_err(|error| error.to_string())?;
        // An opened local world is already the core's route.
        if !local_world {
            select(&self.socket_dir, target_for(address))?;
        }
        Ok(self.socket_dir.clone())
    }
}

fn launcher_command(
    layout: &InstallLayout,
    executable: &Path,
    socket_dir: &Path,
    auth_cache: Option<&Path>,
    upstream_client_cache: bool,
) -> Command {
    let mut command = Command::new(executable);
    command
        .arg("-socket-dir")
        .arg(socket_dir)
        .arg("-control-status")
        .arg("-resource-pack-cache-dir")
        .arg(layout.resource_pack_cache_dir())
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(
            crate::lifecycle::core_health::open_core_log(layout)
                .map_or_else(Stdio::null, Stdio::from),
        );
    // The core refuses to start local worlds without their server binary.
    if executable.with_file_name(LOCAL_SERVER).is_file() {
        command.args(crate::local_worlds::core_args(layout));
    }
    if upstream_client_cache {
        command.arg("-upstream-client-cache");
    }
    if let Some(auth_cache) = auth_cache {
        command.arg("-auth-cache").arg(auth_cache);
    }
    command
}

/// Sends `connect.v1` off the frame thread, waiting a bounded time for the answer.
fn select(socket_dir: &Path, target: ConnectTarget) -> Result<(), String> {
    let (sender, receiver) = crossbeam_channel::bounded(1);
    let socket_dir = socket_dir.to_owned();
    std::thread::spawn(move || {
        let result = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|error| error.to_string())
            .and_then(|runtime| {
                runtime
                    .block_on(launcher_control::connect_target(&socket_dir, &target))
                    .map_err(|error| error.to_string())
            });
        let _ = sender.send(result);
    });
    receiver
        .recv_timeout(SELECT_TIMEOUT)
        .map_err(|_| "the launcher core did not answer".to_owned())?
}

/// Marks a menu address as a gathering's experience ID, joined when selected.
pub(super) const GATHERING_ADDRESS_PREFIX: &str = "gathering/";

/// The `connect.v1` target for a menu address (the proxy's realm and friend
/// prefixes, else a server that gets the default port when it names none).
fn target_for(address: &str) -> ConnectTarget {
    let address = address.trim();
    if let Some(id) = address.strip_prefix(GATHERING_ADDRESS_PREFIX) {
        return ConnectTarget::Gathering(id.to_owned());
    }
    if let Some(id) = address.strip_prefix("realm_id/") {
        return ConnectTarget::Realm(id.to_owned());
    }
    if let Some(xuid) = address.strip_prefix("friend_xuid/") {
        return ConnectTarget::Friend(xuid.to_owned());
    }
    let has_port = address.rsplit_once(':').is_some_and(|(host, port)| {
        port.parse::<u16>().is_ok() && (host.ends_with(']') || !host.contains(':'))
    });
    ConnectTarget::RakNet(if has_port {
        address.to_owned()
    } else if address.contains(':') && !address.starts_with('[') {
        format!("[{address}]:{DEFAULT_PORT}")
    } else {
        format!("{address}:{DEFAULT_PORT}")
    })
}

impl MenuRuntime {
    /// The auth cache a launcher core runs with: only a validated sign-in's.
    fn launcher_auth_cache(&self) -> Option<PathBuf> {
        account::validated_auth_cache(
            &self.layout,
            self.auth_process.as_ref().map(AuthSupervisor::state),
        )
    }

    fn sign_in_in_flight(&self) -> bool {
        matches!(
            self.auth_process.as_ref().map(AuthSupervisor::state),
            Some(AuthState::Checking | AuthState::AwaitingCode { .. })
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn menu_addresses_map_to_connect_targets() {
        assert_eq!(target_for("realm_id/42"), ConnectTarget::Realm("42".into()));
        assert_eq!(
            target_for("gathering/5b0f2bd4-8a8e-4a6e-9d3c-0a1b2c3d4e5f"),
            ConnectTarget::Gathering("5b0f2bd4-8a8e-4a6e-9d3c-0a1b2c3d4e5f".into())
        );
        assert_eq!(
            target_for("friend_xuid/2535"),
            ConnectTarget::Friend("2535".into())
        );
        assert_eq!(
            target_for("play.example.net:19133"),
            ConnectTarget::RakNet("play.example.net:19133".into())
        );
        assert_eq!(
            target_for("play.example.net"),
            ConnectTarget::RakNet("play.example.net:19132".into())
        );
        assert_eq!(
            target_for("[::1]:19134"),
            ConnectTarget::RakNet("[::1]:19134".into())
        );
        assert_eq!(
            target_for("::1"),
            ConnectTarget::RakNet("[::1]:19132".into())
        );
    }

    #[test]
    fn the_launcher_core_serves_control_and_signs_in_only_when_validated() {
        let layout = InstallLayout::scratch("launcher-args");
        let args = |auth: Option<&Path>| -> Vec<String> {
            launcher_command(
                &layout,
                Path::new("/opt/core"),
                Path::new("/run/s"),
                auth,
                false,
            )
            .get_args()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect()
        };
        let offline = args(None);
        assert!(offline.iter().any(|arg| arg == "-control-status"));
        assert!(
            !offline
                .iter()
                .any(|arg| arg == "-upstream" || arg == "-auth-cache")
        );
        let signed_in = args(Some(Path::new("/data/auth.json")));
        assert!(signed_in.iter().any(|arg| arg == "-auth-cache"));
    }
}
