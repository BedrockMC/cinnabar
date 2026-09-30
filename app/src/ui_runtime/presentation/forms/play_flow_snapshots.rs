//! Local-only renders of the launcher's play flow against fixture service data
//! (Realms, friends, featured servers, gatherings, pings, saved servers).
//! Skips without the gitignored carrier; PNGs go to `CINNABAR_FORM_SNAPSHOT_DIR`.

use std::{collections::HashMap, path::PathBuf, sync::Arc};

use ui::DpiScale;

use super::pack_harness::engine_presentation;
use crate::menu::{
    LiveEventCard, LocalWorldCard, MenuFriendCard, MenuGameCard, MenuRealmCard, MenuRuntime,
    MenuScreen, MenuServerCard, MenuView, PingInfo, SavedServer, ServerDetails, auth::AuthState,
};
use crate::ui_runtime::UiRuntime;

/// A solid-coloured PNG with a lighter band, written once per run.
fn art(dir: &std::path::Path, name: &str, size: [u32; 2], color: [u8; 3]) -> String {
    let path = dir.join(format!("{name}.png"));
    let image = image::RgbaImage::from_fn(size[0], size[1], |_, y| {
        let lift = if y < size[1] / 3 { 40 } else { 0 };
        image::Rgba([
            color[0].saturating_add(lift),
            color[1].saturating_add(lift),
            color[2].saturating_add(lift),
            255,
        ])
    });
    image.save(&path).unwrap();
    path.to_string_lossy().into_owned()
}

fn server(name: &str, address: &str, caption: &str, image_path: String) -> MenuServerCard {
    MenuServerCard {
        name: name.to_owned(),
        address: address.to_owned(),
        caption: caption.to_owned(),
        image_path,
        icon: None,
    }
}

fn realm(name: &str, state: &str, member: bool, days_left: i32, expired: bool) -> MenuRealmCard {
    MenuRealmCard {
        name: name.to_owned(),
        state: state.to_owned(),
        target: format!("realm/{name}"),
        address: String::new(),
        owner: if member {
            "Alex".to_owned()
        } else {
            String::new()
        },
        online_players: 3,
        max_players: 10,
        days_left,
        expired,
        member,
    }
}

/// A signed-in view carrying every service feed the play flow shows.
fn fixture_view(dir: &std::path::Path) -> MenuView {
    let mut view = MenuRuntime::new(true, 2, "Steve".to_owned()).view();
    view.auth_state = AuthState::Authenticated;
    view.catalog_loading = false;
    view.featured = vec![
        server(
            "The Hive",
            "geo.hivebedrock.network:19132",
            "Minigames",
            art(dir, "hive", [256, 256], [220, 160, 20]),
        ),
        server(
            "CubeCraft",
            "play.cubecraft.net:19132",
            "Skywars, EggWars",
            art(dir, "cubecraft", [256, 256], [30, 110, 200]),
        ),
    ];
    view.gatherings = vec![server(
        "Minecraft Live",
        "live.example.net:19132",
        "Live event",
        art(dir, "live", [256, 256], [140, 60, 200]),
    )];
    view.realms = vec![
        realm("Steve's Realm", "open", false, 21, false),
        realm("Build Club", "closed", false, 3, false),
        realm("Alex's Realm", "open", true, 0, false),
        realm("ADD ONYXJAVA AS A FRIEND TO JOIN ONYX!", "open", false, 10, false),
    ];
    view.friends = vec![
        MenuFriendCard {
            gamertag: "Alex".to_owned(),
            world_name: "Sky Base".to_owned(),
            members: "2/8 players".to_owned(),
            xuid: "2535400000000001".to_owned(),
        },
        MenuFriendCard {
            gamertag: "Notch".to_owned(),
            world_name: "Survival 2".to_owned(),
            members: "1/8 players".to_owned(),
            xuid: "2535400000000002".to_owned(),
        },
    ];
    view.local_worlds = vec![LocalWorldCard {
        name: "My World".to_owned(),
        game_mode: "Survival".to_owned(),
        date: "9/30/2026".to_owned(),
        size: "12 MB".to_owned(),
    }];
    view.servers = vec![
        SavedServer {
            name: "Home server".to_owned(),
            address: "192.168.1.20:19132".to_owned(),
            favorite: false,
            last_joined_unix: 0,
        },
        SavedServer {
            name: "Test".to_owned(),
            address: "test.example.net:19132".to_owned(),
            favorite: true,
            last_joined_unix: 0,
        },
    ];
    let pong = |players, max_players, ping_ms| PingInfo {
        online: true,
        players,
        max_players,
        ping_ms,
    };
    view.feeds.pings = HashMap::from([
        (
            "geo.hivebedrock.network:19132".to_owned(),
            pong(21_345, 100_000, 40),
        ),
        (
            "play.cubecraft.net:19132".to_owned(),
            pong(8_210, 50_000, 180),
        ),
        ("192.168.1.20:19132".to_owned(), pong(2, 10, 3)),
        ("test.example.net:19132".to_owned(), PingInfo::default()),
    ]);
    view.feeds.details.insert(
        "geo.hivebedrock.network:19132".to_owned(),
        ServerDetails {
            description: "Minigames with friends, every day.".to_owned(),
            news_title: "Season 5".to_owned(),
            news: "A new season of Treasure Wars is live.".to_owned(),
            screenshots: vec![art(dir, "hive_banner", [512, 154], [180, 120, 30])],
            games: vec![MenuGameCard {
                title: "Treasure Wars".to_owned(),
                subtitle: "Teams of four".to_owned(),
                description: "Protect your treasure.".to_owned(),
                image_path: art(dir, "treasure", [128, 128], [200, 60, 60]),
            }],
        },
    );
    view.feeds.profile.gamertag = "Steve".to_owned();
    view.feeds.home.realm_invites = 2;
    view.feeds.home.inbox_unread = 1;
    view.feeds.home.live_event = Some(LiveEventCard {
        button_text: "Learn More".to_owned(),
        caption: "Minecraft Live".to_owned(),
        countdown: false,
        start_unix: 0,
        badge_path: art(dir, "badge", [256, 128], [40, 90, 200]),
        address: "live.example.net:19132".to_owned(),
        route_to_servers: false,
    });
    view
}

fn snapshot(view: &MenuView, name: &str) {
    snapshot_at(view, name, 0);
}

fn snapshot_at(view: &MenuView, name: &str, now_millis: u64) {
    let Some(mut presentation) = engine_presentation() else {
        eprintln!("skipping: UI carrier absent");
        return;
    };
    let mut runtime = UiRuntime::new(1);
    let lang = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../.local/assets/compiled/vanilla-v1.mcbelang");
    if let Some(lang) = std::fs::read(lang)
        .ok()
        .and_then(|bytes| assets::RuntimeLangCatalog::decode(&bytes).ok())
    {
        runtime.set_lang_catalog(Arc::new(lang));
    }
    presentation.sync_menu_artwork(super::super::menu_artwork::view_paths(view));
    let dpi = DpiScale::new(2.0).unwrap();
    for _ in 0..2 {
        presentation.set_menu_view(Some(view.clone()));
        presentation
            .build(&runtime, now_millis, [2560, 1440], dpi)
            .unwrap();
    }
    presentation.set_menu_view(Some(view.clone()));
    let input = presentation
        .build(&runtime, now_millis, [2560, 1440], dpi)
        .unwrap();
    super::snapshot::write(&input, name);
}

// Writes PNGs of each play-flow state (local only).
#[test]
fn snapshot_play_flow() {
    let dir = std::env::temp_dir().join("cinnabar-play-flow-art");
    std::fs::create_dir_all(&dir).unwrap();
    let base = fixture_view(&dir);
    let at = |screen: MenuScreen| {
        let mut view = base.clone();
        view.screen = screen;
        view
    };
    snapshot(&at(MenuScreen::Home), "flow-home");
    snapshot(&at(MenuScreen::Play), "flow-play-worlds");
    snapshot(&at(MenuScreen::Social), "flow-play-realms");
    let mut closed = at(MenuScreen::Social);
    closed.feeds.selected_realm = Some(1);
    snapshot(&closed, "flow-play-realms-closed");
    let mut servers = at(MenuScreen::Servers);
    snapshot(&servers, "flow-play-servers");
    servers.feeds.selected_featured = Some(0);
    snapshot(&servers, "flow-play-servers-featured");
    servers.feeds.select_saved(0);
    snapshot(&servers, "flow-play-servers-saved");
    servers.dialog = Some(crate::menu::MenuDialog::RemoveSaved(0));
    snapshot(&servers, "flow-remove-server");
    snapshot(&at(MenuScreen::Friends), "flow-friends");
    let mut add = at(MenuScreen::AddServer);
    add.name = "Home server".to_owned();
    add.address = "192.168.1.20:19132".to_owned();
    add.editing = Some(0);
    snapshot(&add, "flow-edit-server");
}

// Writes PNGs of the settings screen, the signing-in start screen and two
// frames of the connecting screen's loading bar (local only).
#[test]
fn snapshot_settings_signing_in_and_progress() {
    let dir = std::env::temp_dir().join("cinnabar-play-flow-art");
    std::fs::create_dir_all(&dir).unwrap();
    let base = fixture_view(&dir);
    let mut settings = base.clone();
    settings.screen = MenuScreen::Settings;
    snapshot(&settings, "flow-settings");
    let mut signing_in = base.clone();
    signing_in.screen = MenuScreen::Home;
    signing_in.auth_state = AuthState::Checking;
    snapshot(&signing_in, "flow-home-signing-in");
    let mut connecting = base;
    connecting.connecting = true;
    connecting.message = Some("Connecting...".to_owned());
    snapshot_at(&connecting, "flow-connecting-0", 1_000);
    snapshot_at(&connecting, "flow-connecting-1", 1_350);
}

// The connecting screen's loading bar is a flip-book: later frames paint other
// texels over the same cached layout.
#[test]
fn the_loading_bar_animates_over_its_cached_layout() {
    let Some(mut presentation) = engine_presentation() else {
        return;
    };
    let dir = std::env::temp_dir().join("cinnabar-play-flow-art");
    std::fs::create_dir_all(&dir).unwrap();
    let mut view = fixture_view(&dir);
    view.connecting = true;
    view.message = Some("Connecting...".to_owned());
    let runtime = UiRuntime::new(1);
    let dpi = DpiScale::new(1.0).unwrap();
    let mut frame = |now_millis| {
        presentation.set_menu_view(Some(view.clone()));
        presentation
            .build(&runtime, now_millis, [1280, 720], dpi)
            .unwrap()
    };
    frame(1_000);
    let first = frame(1_000);
    let later = frame(1_350);
    let positions = |input: &render::UiRenderInput| {
        input
            .vertices
            .iter()
            .map(|vertex| vertex.position)
            .collect::<Vec<_>>()
    };
    let uvs = |input: &render::UiRenderInput| {
        input
            .vertices
            .iter()
            .map(|vertex| vertex.uv)
            .collect::<Vec<_>>()
    };
    assert_eq!(positions(&first), positions(&later), "one layout");
    assert_ne!(uvs(&first), uvs(&later), "another frame of the strip");
}

// The disconnect screen words the failure as vanilla does and offers OK, which
// leaves it for the menu.
#[test]
fn the_disconnect_screen_has_a_way_back() {
    let Some(mut presentation) = engine_presentation() else {
        return;
    };
    let dir = std::env::temp_dir().join("cinnabar-play-flow-art");
    std::fs::create_dir_all(&dir).unwrap();
    let mut view = fixture_view(&dir);
    view.screen = MenuScreen::Play;
    view.disconnect_message =
        Some("network session failed: Bedrock session failed: Connection closed".to_owned());
    snapshot(&view, "flow-disconnect");
    presentation.set_menu_view(Some(view));
    let runtime = UiRuntime::new(1);
    let dpi = DpiScale::new(1.0).unwrap();
    let metrics = super::super::TextMetrics::for_viewport([1280, 720], dpi, None);
    let mut nodes = Vec::new();
    let mut next = 1;
    let hits = presentation
        .append_menu(&runtime, &mut nodes, &mut next, metrics, 1280.0, 720.0)
        .unwrap();
    assert!(
        hits.iter()
            .any(|(action, _)| *action == crate::menu::MenuAction::DismissDialog),
        "{hits:?}"
    );
    let texts = super::pack_harness::drawn_texts(&nodes);
    assert!(
        !texts.iter().any(|text| text.contains("session failed")),
        "the raw chain stays in the log: {texts:?}"
    );
}
