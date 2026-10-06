use std::time::Duration;

use super::super::{MenuAction, MenuRuntime, MenuScreen};

fn host() -> MenuRuntime {
    MenuRuntime::new(true, 2, "Host".to_owned())
}

#[test]
fn answers_go_out_in_order_and_the_next_request_takes_the_popup() {
    let mut menu = host();
    menu.push_join_request(1, "Alex".into(), Duration::ZERO);
    menu.push_join_request(2, "Sam".into(), Duration::ZERO);
    assert_eq!(menu.view().join_request_prompt(), Some("Alex"));
    assert_eq!(
        menu.focus_actions(),
        [
            MenuAction::JoinRequest(true),
            MenuAction::JoinRequest(false)
        ]
    );
    menu.activate(MenuAction::JoinRequest(true));
    assert_eq!(menu.view().join_request_prompt(), Some("Sam"));
    // Back on the popup declines, as vanilla's modal escape presses its second button.
    menu.go_back();
    assert_eq!(menu.view().join_request, None);
    assert_eq!(menu.take_join_reply(), Some((1, true)));
    assert_eq!(menu.take_join_reply(), Some((2, false)));
    assert_eq!(menu.take_join_reply(), None);
    assert_eq!(menu.screen(), MenuScreen::Home);
}

#[test]
fn the_toast_opens_the_pause_popup_only_while_a_request_waits() {
    let mut menu = host();
    menu.show_world();
    menu.open_join_requests();
    assert!(!menu.is_visible());
    menu.push_join_request(1, "Alex".into(), Duration::ZERO);
    menu.open_join_requests();
    assert!(menu.is_visible());
    assert_eq!(menu.screen(), MenuScreen::Pause);
    assert_eq!(menu.view().join_request_prompt(), Some("Alex"));
}

#[test]
fn turning_discord_off_drops_requests_and_unsent_answers() {
    let mut menu = host();
    menu.push_join_request(1, "Alex".into(), Duration::ZERO);
    menu.push_join_request(2, "Sam".into(), Duration::ZERO);
    menu.activate(MenuAction::JoinRequest(true));
    menu.clear_join_requests();
    assert_eq!(menu.view().join_request, None);
    assert_eq!(menu.take_join_reply(), None);
}
