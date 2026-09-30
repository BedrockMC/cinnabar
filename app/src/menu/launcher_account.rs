//! [`AccountControl`] over the core's launcher control endpoint. A worker thread
//! polls `events.v1` and `account_status.v1` often and the slow catalog calls
//! (`realms_list.v1`, `friends_list.v1`) rarely and the screen feeds
//! (`featured_servers.v1`, `gatherings.v1`, `profile.v1`) more rarely still, so
//! the menu never blocks on the socket; sign-out requests queue to the same worker.

use std::{
    collections::HashSet,
    path::PathBuf,
    sync::{Arc, Mutex},
    thread,
    time::{Duration, Instant},
};

use bevy::prelude::Resource;
use crossbeam_channel::{Receiver, Sender, bounded};
use protocol::launcher_control::{
    self, Account, AuthState as CoreAuth, ConnectProgress, ConnectStage, FeaturedServer, Friend,
    Gathering, Home, Message, MessageEvent, Profile, Realm, ServerPing,
};

use super::account_control::{AccountControl, AccountEvent};
use super::view::{
    ButtonArt, InboxItem, JoinStage, LiveEventCard, MenuGameCard, MenuHome, MenuProfile, PingInfo,
    ServerDetails,
};
use super::{AuthState, MenuFriendCard, MenuRealmCard, MenuServerCard};

/// How often auth state and events refresh.
const EVENT_INTERVAL: Duration = Duration::from_secs(1);
/// How often events refresh while a join is under way, so its progress bar moves smoothly.
const JOIN_EVENT_INTERVAL: Duration = Duration::from_millis(250);
/// How often the catalog lists refresh (they can take tens of seconds).
const CATALOG_INTERVAL: Duration = Duration::from_secs(30);
/// How often the screen feeds are read; the core answers from its catalog cache
/// and refreshes upstream on its own schedule, so this only picks up fresh data.
const FEED_INTERVAL: Duration = Duration::from_secs(30);
/// How soon a feed that failed is asked again.
const FEED_RETRY: Duration = Duration::from_secs(15);
/// How often shown server rows are pinged.
const PING_INTERVAL: Duration = Duration::from_secs(15);

#[derive(Default)]
struct Snapshot {
    account: Option<Account>,
    realms: Option<Vec<Realm>>,
    friends: Option<Vec<Friend>>,
    /// Delivered once per fetch.
    featured: Option<Vec<FeaturedServer>>,
    gatherings: Option<Vec<Gathering>>,
    profile: Option<Profile>,
    ping_targets: Vec<String>,
    pings: Option<Vec<ServerPing>>,
    home: Option<Home>,
    events: Vec<AccountEvent>,
    last_disconnect: Option<u64>,
    connect: Option<ConnectProgress>,
    /// The menu is connecting; the worker polls events faster and defers slow calls.
    joining: bool,
}

/// The menu's link to a running core's launcher control endpoint.
#[derive(Resource)]
pub(crate) struct LauncherAccount {
    snapshot: Arc<Mutex<Snapshot>>,
    sign_out: Sender<()>,
    socket_dir: PathBuf,
}

impl LauncherAccount {
    /// Start polling the control endpoint under `socket_dir`; the worker stops
    /// when this is dropped.
    pub(crate) fn new(socket_dir: PathBuf) -> Self {
        let snapshot = Arc::new(Mutex::new(Snapshot::default()));
        let (sign_out, requests) = bounded(1);
        let shared = Arc::clone(&snapshot);
        let worker_dir = socket_dir.clone();
        thread::spawn(move || poll(&worker_dir, &shared, &requests));
        Self {
            snapshot,
            sign_out,
            socket_dir,
        }
    }

    /// The control endpoint directory this link polls.
    pub(crate) fn socket_dir(&self) -> &std::path::Path {
        &self.socket_dir
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
    let mut feed_due = Instant::now();
    let mut ping_due = Instant::now();
    let mut reported = HashSet::new();
    loop {
        let joining = shared
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .joining;
        let interval = if joining {
            JOIN_EVENT_INTERVAL
        } else {
            EVENT_INTERVAL
        };
        match requests.recv_timeout(interval) {
            Ok(()) => {
                let _ = runtime.block_on(launcher_control::sign_out(socket_dir));
            }
            Err(crossbeam_channel::RecvTimeoutError::Disconnected) => return,
            Err(crossbeam_channel::RecvTimeoutError::Timeout) => {}
        }
        let events = runtime
            .block_on(launcher_control::poll_events(socket_dir))
            .ok();
        // Slow calls wait out a join so they never stall its progress.
        let catalog = (!joining && Instant::now() >= catalog_due).then(|| {
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
        let feeds = (!joining && Instant::now() >= feed_due).then(|| {
            let mut failed = false;
            let home = settle(
                "home",
                runtime.block_on(launcher_control::home(socket_dir)),
                &mut failed,
            );
            if let Some(home) = &home {
                report_impressions(&runtime, socket_dir, home, &mut reported);
            }
            let featured = settle(
                "featured servers",
                runtime.block_on(launcher_control::list_featured_servers(socket_dir)),
                &mut failed,
            );
            let gatherings = settle(
                "gatherings",
                runtime.block_on(launcher_control::list_gatherings(socket_dir)),
                &mut failed,
            );
            let profile = settle(
                "profile",
                runtime.block_on(launcher_control::profile(socket_dir)),
                &mut failed,
            );
            feed_due = Instant::now() + if failed { FEED_RETRY } else { FEED_INTERVAL };
            (home, featured, gatherings, profile)
        });
        let targets = shared
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .ping_targets
            .clone();
        let pings = (!targets.is_empty() && Instant::now() >= ping_due).then(|| {
            ping_due = Instant::now() + PING_INTERVAL;
            settle(
                "ping",
                runtime.block_on(launcher_control::ping_servers(socket_dir, &targets)),
                &mut false,
            )
        });
        let mut snapshot = shared.lock().unwrap_or_else(|poison| poison.into_inner());
        if let Some(Some(pings)) = pings {
            snapshot.pings = Some(pings);
        }
        if let Some((home, featured, gatherings, profile)) = feeds {
            snapshot.home = home.or(snapshot.home.take());
            snapshot.featured = featured.or(snapshot.featured.take());
            snapshot.gatherings = gatherings.or(snapshot.gatherings.take());
            snapshot.profile = profile.or(snapshot.profile.take());
        }
        if let Some(events) = events {
            if let Some(disconnect) = events.disconnect
                && snapshot.last_disconnect != Some(disconnect.sequence)
            {
                // The first poll only records the standing disconnect.
                if snapshot.last_disconnect.is_some() {
                    // An empty message reads as vanilla's no-reason line.
                    let reason = disconnect.message.trim().to_owned();
                    snapshot.events.push(AccountEvent::Disconnected { reason });
                }
                snapshot.last_disconnect = Some(disconnect.sequence);
            }
            snapshot.last_disconnect.get_or_insert(0);
            snapshot.connect = events.connect;
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

/// A feed's value, or `None` after logging which feed failed; the core logs the
/// upstream cause, redacted.
fn settle<T, E: std::fmt::Display>(
    feed: &str,
    result: Result<T, E>,
    failed: &mut bool,
) -> Option<T> {
    match result {
        Ok(value) => Some(value),
        Err(error) => {
            *failed = true;
            bevy::log::warn!(feed, %error, "launcher feed failed; retrying soon");
            None
        }
    }
}

/// Button-art surfaces the start screen shows, reported once per message instance.
const SHOWN_SURFACES: [&str; 2] = ["PlayButton", "MarketplaceButton"];

fn report_impressions(
    runtime: &tokio::runtime::Runtime,
    socket_dir: &std::path::Path,
    home: &Home,
    reported: &mut HashSet<String>,
) {
    for message in &home.messages {
        if !SHOWN_SURFACES.contains(&message.surface.as_str())
            || !reported.insert(message.instance_id.clone())
        {
            continue;
        }
        let event = MessageEvent {
            event_type: "Impression".to_owned(),
            instance_id: message.instance_id.clone(),
            report_id: message.report_id.clone(),
            button_id: String::new(),
        };
        let _ = runtime.block_on(launcher_control::report_message_event(socket_dir, &event));
    }
}

/// The start screen's view of the core's home feed.
fn menu_home(home: &Home, now_unix: i64) -> MenuHome {
    let art = |surface: &str| {
        home.messages
            .iter()
            .find(|message| message.surface == surface)
            .map(button_art)
    };
    let live_event = home
        .live_events
        .iter()
        .find(|event| event.end_unix == 0 || now_unix < event.end_unix)
        .map(|event| LiveEventCard {
            button_text: event.button_text.clone(),
            caption: event.caption_text.clone(),
            countdown: event.caption_countdown,
            start_unix: event.start_unix,
            badge_path: event.badge.path.clone(),
            address: event.address.clone(),
            route_to_servers: event.route_to_servers,
        });
    MenuHome {
        play_art: art("PlayButton"),
        store_art: art("MarketplaceButton"),
        inbox_unread: home.inbox.unread,
        realm_invites: home.realm_invites,
        live_event,
        persona_head: home.persona_head.path.clone(),
        inbox: home
            .messages
            .iter()
            .filter(|message| message.surface == "InboxMessage")
            .map(|message| InboxItem {
                header: message.header.clone(),
                body: message.body.clone(),
                category: message.category.clone(),
                unread: !message.status.eq_ignore_ascii_case("read"),
            })
            .collect(),
    }
}

/// Sorts a tile's images into the button's layers by their ids (hover, foreground).
fn button_art(message: &Message) -> ButtonArt {
    let mut art = ButtonArt {
        banner: message.banner.clone(),
        ..ButtonArt::default()
    };
    for image in message.images.iter().filter(|image| !image.path.is_empty()) {
        let id = image.id.to_ascii_lowercase();
        let hover = id.contains("hover");
        let foreground = id.contains("fore") || id.contains("fg");
        let slot = match (hover, foreground) {
            (true, true) => &mut art.hover_foreground,
            (true, false) => &mut art.hover_background,
            (false, true) => &mut art.default_foreground,
            (false, false) => &mut art.default_background,
        };
        if slot.is_empty() {
            *slot = image.path.clone();
        }
    }
    art
}

/// The core's account state as the menu's sign-in state; an offline core
/// knows nothing about the account, so the auth supervisor's state stands.
fn auth_state(account: &Account) -> Option<AuthState> {
    Some(match account.state {
        CoreAuth::Offline => return None,
        CoreAuth::SignedOut => AuthState::SignedOut,
        CoreAuth::AwaitingCode => AuthState::AwaitingCode {
            uri: account.verification_uri.clone().unwrap_or_default(),
            code: account.user_code.clone().unwrap_or_default(),
        },
        CoreAuth::SignedIn => AuthState::Authenticated,
        CoreAuth::Failed => AuthState::Failed(account.reason.clone().unwrap_or_default()),
    })
}

fn join_stage(progress: &ConnectProgress) -> JoinStage {
    match progress.stage {
        ConnectStage::Realm => JoinStage::Realm,
        ConnectStage::Connecting => JoinStage::Connecting,
        ConnectStage::Packs => JoinStage::Packs {
            done: progress.packs_done,
            total: progress.packs_total,
            received_bytes: progress.received_bytes,
            total_bytes: progress.total_bytes,
        },
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
        self.with(|snapshot| snapshot.account.as_ref().and_then(auth_state))
    }

    fn join_stage(&mut self) -> Option<JoinStage> {
        self.with(|snapshot| snapshot.connect.as_ref().map(join_stage))
    }

    fn set_joining(&mut self, joining: bool) {
        self.with(|snapshot| snapshot.joining = joining);
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
                        owner: realm.owner.clone(),
                        online_players: realm.online_players,
                        max_players: realm.max_players,
                        days_left: realm.days_left,
                        expired: realm.expired,
                        member: realm.member,
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
        let queued = self.sign_out.try_send(()).is_ok();
        if queued {
            // The signed-in lists and status are stale from here on.
            self.with(|snapshot| {
                snapshot.account = None;
                snapshot.realms = None;
                snapshot.friends = None;
            });
        }
        queued
    }

    fn poll_event(&mut self) -> Option<AccountEvent> {
        self.with(|snapshot| (!snapshot.events.is_empty()).then(|| snapshot.events.remove(0)))
    }

    fn featured(&mut self) -> Option<Vec<(MenuServerCard, ServerDetails)>> {
        let servers = self.with(|snapshot| snapshot.featured.take())?;
        Some(servers.iter().map(featured_card).collect())
    }

    fn gatherings(&mut self) -> Option<Vec<(MenuServerCard, ServerDetails)>> {
        let gatherings = self.with(|snapshot| snapshot.gatherings.take())?;
        Some(
            gatherings
                .iter()
                .filter(|gathering| !gathering.address.is_empty())
                .map(|gathering| {
                    let card = MenuServerCard {
                        name: gathering.name.clone(),
                        address: gathering.address.clone(),
                        caption: gathering.caption.clone(),
                        image_path: gathering.image.path.clone(),
                        icon: None,
                    };
                    let details = ServerDetails {
                        description: gathering.description.clone(),
                        ..ServerDetails::default()
                    };
                    (card, details)
                })
                .collect(),
        )
    }

    fn home(&mut self) -> Option<MenuHome> {
        let home = self.with(|snapshot| snapshot.home.take())?;
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |elapsed| elapsed.as_secs() as i64);
        Some(menu_home(&home, now))
    }

    fn set_ping_targets(&mut self, mut targets: Vec<String>) {
        targets.truncate(64);
        self.with(|snapshot| {
            if snapshot.ping_targets != targets {
                snapshot.ping_targets = targets;
            }
        });
    }

    fn pings(&mut self) -> Option<Vec<(String, PingInfo)>> {
        let pings = self.with(|snapshot| snapshot.pings.take())?;
        Some(
            pings
                .into_iter()
                .map(|ping| {
                    let info = PingInfo {
                        online: ping.online,
                        players: ping.players,
                        max_players: ping.max_players,
                        ping_ms: ping.ping_ms,
                    };
                    (ping.address, info)
                })
                .collect(),
        )
    }

    fn profile(&mut self) -> Option<MenuProfile> {
        let profile = self.with(|snapshot| snapshot.profile.take())?;
        Some(MenuProfile {
            gamertag: profile.gamertag,
            picture_path: profile.gamerpic.path,
            real_name: profile.real_name,
            presence: profile.presence_text,
            gamerscore: profile.gamerscore,
            friends: profile.friends,
            followers: profile.followers,
        })
    }
}

fn featured_card(server: &FeaturedServer) -> (MenuServerCard, ServerDetails) {
    let card = MenuServerCard {
        name: server.name.clone(),
        address: server.address.clone(),
        caption: server.caption.clone(),
        image_path: server.logo.path.clone(),
        icon: None,
    };
    let details = ServerDetails {
        description: server.description.clone(),
        news_title: server.news_title.clone(),
        news: server.news.clone(),
        screenshots: server
            .screenshots
            .iter()
            .filter(|shot| !shot.path.is_empty())
            .map(|shot| shot.path.clone())
            .collect(),
        games: server
            .games
            .iter()
            .map(|game| MenuGameCard {
                title: game.title.clone(),
                subtitle: game.subtitle.clone(),
                description: game.description.clone(),
                image_path: game.image.path.clone(),
            })
            .collect(),
    };
    (card, details)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tile_images_sort_into_button_layers() {
        let image = |id: &str| protocol::launcher_control::MessageImage {
            id: id.into(),
            url: String::new(),
            path: format!("/art/{id}.img"),
        };
        let message = Message {
            surface: "PlayButton".into(),
            banner: "New".into(),
            images: vec![
                image("background"),
                image("hoverForeground"),
                image("hover"),
            ],
            ..Message::default()
        };
        let home = Home {
            messages: vec![message],
            realm_invites: 2,
            ..Home::default()
        };
        let menu = menu_home(&home, 0);
        let art = menu.play_art.expect("play art");
        assert_eq!(art.default_background, "/art/background.img");
        assert_eq!(art.hover_foreground, "/art/hoverForeground.img");
        assert_eq!(art.hover_background, "/art/hover.img");
        assert_eq!(art.banner, "New");
        assert!(menu.store_art.is_none());
        assert_eq!(menu.realm_invites, 2);
    }

    #[test]
    fn featured_servers_split_into_cards_and_details() {
        let server = FeaturedServer {
            name: "S".into(),
            address: "a.test:19132".into(),
            news: "Update".into(),
            screenshots: vec![
                protocol::launcher_control::Artwork {
                    url: "https://a.test/s.png".into(),
                    path: String::new(),
                },
                protocol::launcher_control::Artwork {
                    url: "https://a.test/t.png".into(),
                    path: "/art/t.img".into(),
                },
            ],
            ..FeaturedServer::default()
        };
        let (card, details) = featured_card(&server);
        assert_eq!(card.address, "a.test:19132");
        assert_eq!(details.news, "Update");
        assert_eq!(details.screenshots, vec!["/art/t.img".to_owned()]);
    }

    #[test]
    fn core_connect_stages_map_to_join_stages() {
        let progress = |stage| ConnectProgress {
            stage,
            packs_done: 1,
            packs_total: 2,
            received_bytes: 3,
            total_bytes: 4,
        };
        assert_eq!(join_stage(&progress(ConnectStage::Realm)), JoinStage::Realm);
        assert_eq!(
            join_stage(&progress(ConnectStage::Connecting)),
            JoinStage::Connecting
        );
        assert_eq!(
            join_stage(&progress(ConnectStage::Packs)),
            JoinStage::Packs {
                done: 1,
                total: 2,
                received_bytes: 3,
                total_bytes: 4
            }
        );
    }

    #[test]
    fn core_account_states_map_to_menu_sign_in_states() {
        let account = |state| Account {
            state,
            verification_uri: Some("https://aka.ms/remoteconnect".into()),
            user_code: Some("ABCD".into()),
            gamertag: None,
            reason: Some("expired".into()),
        };
        assert_eq!(auth_state(&account(CoreAuth::Offline)), None);
        assert_eq!(
            auth_state(&account(CoreAuth::SignedOut)),
            Some(AuthState::SignedOut)
        );
        assert_eq!(
            auth_state(&account(CoreAuth::AwaitingCode)),
            Some(AuthState::AwaitingCode {
                uri: "https://aka.ms/remoteconnect".into(),
                code: "ABCD".into()
            })
        );
        assert_eq!(
            auth_state(&account(CoreAuth::SignedIn)),
            Some(AuthState::Authenticated)
        );
        assert_eq!(
            auth_state(&account(CoreAuth::Failed)),
            Some(AuthState::Failed("expired".into()))
        );
    }
}
