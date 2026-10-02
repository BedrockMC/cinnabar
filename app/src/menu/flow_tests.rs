//! The death screen opens once per death and hands respawn to the session.
use super::{MenuAction, MenuRuntime, MenuScreen};

#[test]
fn death_screen_opens_once_per_death_and_requests_respawn() {
    let mut menu = MenuRuntime::new(false, 2, "Steve".to_owned());
    menu.open_death();
    assert!(menu.is_visible());
    assert_eq!(menu.view().screen, MenuScreen::Death);
    menu.activate(MenuAction::Respawn);
    assert!(!menu.is_visible());
    assert!(menu.take_respawn_request());
    assert!(!menu.take_respawn_request(), "the request is consumed once");
    menu.open_death();
    assert!(
        !menu.is_visible(),
        "the same death does not reopen the screen"
    );
    menu.note_player_alive();
    menu.open_death();
    assert!(menu.is_visible(), "a later death shows it again");
}

#[test]
fn local_world_choices_reach_the_worlds_module() {
    let mut menu = MenuRuntime::new(true, 2, "Steve".to_owned());
    menu.activate(MenuAction::PlayLocalWorld(0));
    assert_eq!(
        menu.take_local_world_request(),
        None,
        "no world at that index"
    );
    menu.set_local_worlds(vec![super::LocalWorldCard::default()]);
    menu.activate(MenuAction::PlayLocalWorld(0));
    assert_eq!(menu.take_local_world_request(), Some(0));
}

// The port box edits its own value and joins the host as `host:port`; a typed port wins.
#[test]
fn server_draft_joins_the_separate_port_box() {
    let mut menu = MenuRuntime::new(true, 2, "Steve".to_owned());
    menu.activate(MenuAction::PlayAddServer);
    assert_eq!(menu.view().port, "19132");
    assert_eq!(menu.draft_endpoint(), "", "an empty host stays empty");
    menu.activate(MenuAction::AddPort);
    assert_eq!(menu.view().field, Some(super::MenuField::Port));
    menu.address = "play.example".to_owned();
    menu.port = "19133".to_owned();
    assert_eq!(menu.draft_endpoint(), "play.example:19133");
    menu.address = "play.example:25565".to_owned();
    assert_eq!(menu.draft_endpoint(), "play.example:25565");
    menu.address = "::1".to_owned();
    assert_eq!(menu.draft_endpoint(), "[::1]:19133");
    menu.address = "play.example".to_owned();
    menu.port.clear();
    assert_eq!(menu.draft_endpoint(), "play.example");
}
