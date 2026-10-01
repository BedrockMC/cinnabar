
use super::*;
use crate::menu::MenuRuntime;
use json_ui::RectOut;

fn view(screen: MenuScreen) -> MenuView {
    let mut view = MenuRuntime::new(true, 2, "Steve".to_owned()).view();
    view.screen = screen;
    view.auth_state = AuthState::SignedOut;
    view
}

fn region(kind: HitKind, pressed: Option<&str>) -> HitRegion {
    let rect = RectOut {
        x: 0.0,
        y: 0.0,
        w: 10.0,
        h: 10.0,
    };
    HitRegion {
        key: "/screen/button".to_owned(),
        name: "button".to_owned(),
        kind,
        rect,
        clip: rect,
        layer: 0,
        order: 0,
        pressed: pressed.map(str::to_owned),
        control_name: None,
        collection_index: None,
        collection: None,
        enabled: true,
        checked: None,
        max_length: None,
        group_index: None,
        renderer: None,
        drag_axes: [false; 2],
        sound: None,
        input: Default::default(),
        focus: None,
        widget: Default::default(),
        collections: Vec::new(),
        modal_root: None,
    }
}

fn reference(view: &MenuView) -> Option<&'static str> {
    screen_data(view, &|_| None).map(|screen| screen.reference)
}

#[test]
fn the_pause_store_button_names_the_server_store() {
    assert_eq!(server_store_text(&|_| None), "Server Store");
}

#[test]
fn the_version_reads_as_the_release_client_shows_it() {
    assert_eq!(version_label("1.26.50"), "v26.50");
    assert_eq!(version_label("26.60"), "v26.60");
}

#[test]
fn menu_states_open_their_vanilla_screens() {
    assert_eq!(
        reference(&view(MenuScreen::Pause)),
        Some("pause.pause_screen")
    );
    assert_eq!(
        reference(&view(MenuScreen::Home)),
        Some("start.start_screen")
    );
    assert_eq!(
        reference(&view(MenuScreen::Death)),
        Some("death.death_screen")
    );
    assert_eq!(
        reference(&view(MenuScreen::Servers)),
        Some("play.play_screen")
    );
    let mut connecting = view(MenuScreen::Play);
    connecting.connecting = true;
    assert_eq!(
        reference(&connecting),
        Some("progress.world_loading_progress_screen")
    );
    connecting.feeds.join = crate::menu::JoinProgress::new(crate::menu::JoinKind::Realm);
    assert_eq!(
        reference(&connecting),
        Some("progress.realms_stories_loading_progress_screen")
    );
    let mut dropped = view(MenuScreen::Play);
    dropped.disconnect_message = Some("Kicked".into());
    assert_eq!(reference(&dropped), Some("disconnect.disconnect_screen"));
    let mut code = view(MenuScreen::Home);
    code.auth_state = AuthState::AwaitingCode {
        uri: "https://x".into(),
        code: "ABC".into(),
    };
    assert_eq!(
        reference(&code),
        Some("xbl_console_signin.xbl_console_signin")
    );
}

/// A local world's loading screen wins over the plain connecting screen and cancels the open.
#[test]
fn local_world_progress_opens_the_loading_screen_with_cancel() {
    let mut opening = view(MenuScreen::Play);
    opening.connecting = true;
    opening.local.progress = Some(crate::local_worlds::Progress::connecting("Home"));
    assert_eq!(reference(&opening), Some(LOCAL_WORLD_PROGRESS_SCREEN));
    let cancel = action_for(&opening, &region(HitKind::Button, Some("button.menu_exit")));
    assert_eq!(
        cancel,
        Some(MenuAction::LocalWorld(crate::menu::LocalWorldAction::Back))
    );
}

#[test]
fn pressed_buttons_map_to_menu_actions() {
    let pause = view(MenuScreen::Pause);
    let press = |view: &MenuView, id: &str| action_for(view, &region(HitKind::Button, Some(id)));
    assert_eq!(
        press(&pause, "button.menu_continue"),
        Some(MenuAction::PauseResume)
    );
    assert_eq!(
        press(&pause, "button.menu_quit"),
        Some(MenuAction::PauseDisconnect)
    );
    let death = view(MenuScreen::Death);
    assert_eq!(
        press(&death, "button.respawn_button"),
        Some(MenuAction::Respawn)
    );
    let home = view(MenuScreen::Home);
    assert_eq!(
        press(&home, "button.menu_exit"),
        Some(MenuAction::OpenExitDialog)
    );
    let mut edit = region(
        HitKind::Button,
        Some("button.menu_network_server_world_edit"),
    );
    edit.collection_index = Some(3);
    assert_eq!(
        action_for(&view(MenuScreen::Servers), &edit),
        Some(MenuAction::EditSaved(3))
    );
}

#[test]
fn the_start_screen_marketplace_button_opens_the_store_and_its_presses_route_to_it() {
    let home = view(MenuScreen::Home);
    assert_eq!(
        action_for(&home, &region(HitKind::Button, Some("button.menu_store"))),
        Some(MenuAction::Store(crate::store::StoreAction::Open))
    );
    let mut store = view(MenuScreen::Store);
    assert!(
        reference(&store).is_none(),
        "no engine screen until the store publishes"
    );
    store.store = Some(std::sync::Arc::new(crate::store::StoreSnapshot::empty()));
    assert_eq!(
        reference(&store),
        Some("store_layout.store_data_driven_screen")
    );
    assert_eq!(
        action_for(&store, &region(HitKind::Button, Some("button.menu_exit"))),
        Some(MenuAction::Store(crate::store::StoreAction::Back))
    );
}

#[test]
fn radio_tabs_pick_play_tabs_and_settings_sections() {
    let mut tab = region(HitKind::Toggle, None);
    tab.control_name = Some("navigation_tab".into());
    tab.group_index = Some(1);
    assert_eq!(
        action_for(&view(MenuScreen::Play), &tab),
        Some(MenuAction::Navigate(MenuScreen::Social))
    );
    tab.group_index = Some(8);
    assert_eq!(
        action_for(&view(MenuScreen::Settings), &tab),
        Some(MenuAction::SettingsSection(8))
    );
}

#[test]
fn fullscreen_toggle_binds_and_changes_the_current_window_mode() {
    let mut view = view(MenuScreen::Settings);
    view.fullscreen = false;
    let mut toggle = region(HitKind::Toggle, None);
    toggle.control_name = Some("full_screen".into());
    assert_eq!(
        action_for(&view, &toggle),
        Some(MenuAction::SettingsFullscreen(true))
    );
    view.fullscreen = true;
    assert_eq!(
        action_for(&view, &toggle),
        Some(MenuAction::SettingsFullscreen(false))
    );
    let data = screen_data(&view, &|_| None).unwrap().data;
    let toggle = json_ui::ResolvedControl {
        name: "full_screen".into(),
        control_type: Some("toggle".into()),
        base: None,
        unresolved_base: None,
        properties: serde_json::json!({"bindings": [
            {"binding_name": "#full_screen", "binding_name_override": "#toggle_state"},
            {"binding_name": "#full_screen_enabled", "binding_name_override": "#enabled"}
        ]})
        .as_object()
        .unwrap()
        .iter()
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect(),
        children: Vec::new(),
        factory: None,
    };
    let bound = json_ui::bind(&toggle, &data, &json_ui::EmptyLibrary);
    assert_eq!(
        bound.properties.get("#toggle_state"),
        Some(&Value::Bool(true))
    );
    assert_eq!(bound.properties.get("#enabled"), Some(&Value::Bool(true)));
}

#[test]
fn sliders_split_into_their_settings_values() {
    let mut view = MenuRuntime::new(true, 2, "Player".to_owned()).view();
    view.gui_scale_choices = ui::DesktopGuiScale::for_window([1920, 1080])
        .offsets()
        .collect();
    let mut slider = region(HitKind::Slider, None);
    slider.control_name = Some("gui_scale".to_owned());
    let scale = slider_actions(&view, &slider).unwrap();
    assert_eq!(
        scale,
        vec![
            MenuAction::SettingsScale(-2),
            MenuAction::SettingsScale(-1),
            MenuAction::SettingsScale(0)
        ]
    );
    slider.control_name = Some("music_volume".to_owned());
    let music = slider_actions(&view, &slider).unwrap();
    assert_eq!(music.len(), usize::from(VOLUME_STEPS));
    assert_eq!(music[0], MenuAction::SettingsVolume(1, 0));
    assert_eq!(music.last(), Some(&MenuAction::SettingsVolume(1, 100)));
    slider.control_name = Some("fov".to_owned());
    assert!(slider_actions(&view, &slider).is_none());
}

#[test]
fn addresses_split_into_the_ip_and_port_boxes() {
    assert_eq!(
        split_address("play.example:19133"),
        ("play.example".into(), "19133".into())
    );
    assert_eq!(split_address("[::1]:19132"), ("::1".into(), "19132".into()));
    assert_eq!(split_address("host"), ("host".into(), "19132".into()));
}
