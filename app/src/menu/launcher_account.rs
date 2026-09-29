//! [`AccountControl`] over the core's launcher control endpoint. A worker thread
//! polls `events.v1` and `account_status.v1` often and the slow catalog calls
//! (`realms_list.v1`, `friends_list.v1`) rarely, so the menu never blocks on
//! the socket; sign-out requests queue to the same worker.

use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
    thread,
    time::{Duration, Instant},
};

use bevy::prelude::Resource;
use crossbeam_channel::{Receiver, Sender, bounded};
use protocol::launcher_control::{self, Account, AuthState as CoreAuth, Friend, Realm};

use super::account_control::{AccountControl, AccountEvent};
use super::{AuthState, MenuFriendCard, MenuRealmCard};

/// How often auth state and events refresh.
const EVENT_INTERVAL: Duration = Duration::from_secs(1);
/// How often the catalog lists refresh (they can take tens of seconds).
const CATALOG_INTERVAL: Duration = Duration::from_secs(30);

#[derive(Default)]
struct Snapshot {
    account: Option<Account>,
    realms: Option<Vec<Realm>>,
    friends: Option<Vec<Friend>>,
    events: Vec<AccountEvent>,
    last_disconnect: Option<u64>,
}

/// The menu's link to a running core's launcher control endpoint.
#[derive(Resource)]
pub(crate) struct LauncherAccount {
    snapshot: Arc<Mutex<Snapshot>>,
    sign_out: Sender<()>,
}

impl LauncherAccount {
    /// Start polling the control endpoint under `socket_dir`; the worker stops
    /// when this is dropped.
    #[cfg_attr(
        not(test),
        allow(dead_code, reason = "inserted once the launcher core serves control")
    )]
    pub(crate) fn new(socket_dir: PathBuf) -> Self {
        let snapshot = Arc::new(Mutex::new(Snapshot::default()));
        let (sign_out, requests) = bounded(1);
        let shared = Arc::clone(&snapshot);
        thread::spawn(move || poll(&socket_dir, &shared, &requests));
        Self { snapshot, sign_out }
    }

    fn with<T>(&self, read: impl FnOnce(&mut Snapshot) -> T) -> T {
        let mut snapshot = self
            .snapshot
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        read(&mut snapshot)
    }
}

fn poll(socket_dir: &std::path::Path, shared: &Mutex<Snapshot>, requests: &Receiver<()>) {
    let Ok(runtime) = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    else {
        return;
    };
    let mut catalog_due = Instant::now();
    loop {
        match requests.recv_timeout(EVENT_INTERVAL) {
            Ok(()) => {
                let _ = runtime.block_on(launcher_control::sign_out(socket_dir));
            }
            Err(crossbeam_channel::RecvTimeoutError::Disconnected) => return,
            Err(crossbeam_channel::RecvTimeoutError::Timeout) => {}
        }
        let events = runtime
            .block_on(launcher_control::poll_events(socket_dir))
            .ok();
        let catalog = (Instant::now() >= catalog_due).then(|| {
            catalog_due = Instant::now() + CATALOG_INTERVAL;
            (
                runtime
                    .block_on(launcher_control::list_realms(socket_dir))
                    .ok(),
                runtime
                    .block_on(launcher_control::list_friends(socket_dir))
                    .ok(),
            )
        });
        let mut snapshot = shared.lock().unwrap_or_else(|poison| poison.into_inner());
        if let Some(events) = events {
            if let Some(disconnect) = events.disconnect
                && snapshot.last_disconnect != Some(disconnect.sequence)
            {
                // The first poll only records the standing disconnect.
                if snapshot.last_disconnect.is_some() {
                    snapshot.events.push(AccountEvent::Disconnected {
                        reason: disconnect.message,
                    });
                }
                snapshot.last_disconnect = Some(disconnect.sequence);
            }
            snapshot.last_disconnect.get_or_insert(0);
            snapshot.account = Some(events.auth);
        }
        if let Some((realms, friends)) = catalog {
            if realms.is_some() {
                snapshot.realms = realms;
            }
            if friends.is_some() {
                snapshot.friends = friends;
            }
        }
    }
}

/// The core's account state as the menu's sign-in state.
fn auth_state(account: &Account) -> AuthState {
    match account.state {
        CoreAuth::Offline | CoreAuth::SignedOut => AuthState::SignedOut,
        CoreAuth::AwaitingCode => AuthState::AwaitingCode {
            uri: account.verification_uri.clone().unwrap_or_default(),
            code: account.user_code.clone().unwrap_or_default(),
        },
        CoreAuth::SignedIn => AuthState::Authenticated,
        CoreAuth::Failed => AuthState::Failed(account.reason.clone().unwrap_or_default()),
    }
}

fn friend_card(friend: &Friend) -> MenuFriendCard {
    let members = if friend.max_members > 0 {
        format!("{}/{} players", friend.members, friend.max_members)
    } else {
        format!("{} players", friend.members)
    };
    MenuFriendCard {
        gamertag: friend.gamertag.clone(),
        world_name: friend.world_name.clone(),
        members,
        xuid: friend.xuid.clone(),
    }
}

impl AccountControl for LauncherAccount {
    fn account_status(&mut self) -> Option<AuthState> {
        self.with(|snapshot| snapshot.account.as_ref().map(auth_state))
    }

    fn realms(&mut self) -> Option<Vec<MenuRealmCard>> {
        self.with(|snapshot| {
            snapshot.realms.as_ref().map(|realms| {
                realms
                    .iter()
                    .map(|realm| MenuRealmCard {
                        name: realm.name.clone(),
                        state: realm.state.clone(),
                        target: realm.target.clone(),
                        address: realm.address.clone().unwrap_or_default(),
                    })
                    .collect()
            })
        })
    }

    fn friends(&mut self) -> Option<Vec<MenuFriendCard>> {
        self.with(|snapshot| {
            snapshot
                .friends
                .as_ref()
                .map(|friends| friends.iter().map(friend_card).collect())
        })
    }

    fn sign_out(&mut self) -> bool {
        self.sign_out.try_send(()).is_ok()
    }

    fn poll_event(&mut self) -> Option<AccountEvent> {
        self.with(|snapshot| (!snapshot.events.is_empty()).then(|| snapshot.events.remove(0)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn core_account_states_map_to_menu_sign_in_states() {
        let account = |state| Account {
            state,
            verification_uri: Some("https://aka.ms/remoteconnect".into()),
            user_code: Some("ABCD".into()),
            gamertag: None,
            reason: Some("expired".into()),
        };
        assert_eq!(
            auth_state(&account(CoreAuth::Offline)),
            AuthState::SignedOut
        );
        assert_eq!(
            auth_state(&account(CoreAuth::AwaitingCode)),
            AuthState::AwaitingCode {
                uri: "https://aka.ms/remoteconnect".into(),
                code: "ABCD".into()
            }
        );
        assert_eq!(
            auth_state(&account(CoreAuth::SignedIn)),
            AuthState::Authenticated
        );
        assert_eq!(
            auth_state(&account(CoreAuth::Failed)),
            AuthState::Failed("expired".into())
        );
    }
}
