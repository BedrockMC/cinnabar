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
