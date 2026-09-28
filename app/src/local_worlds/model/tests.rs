use protocol::world_control::{Difficulty, GameMode, Generator, World, WorldState, WorldStatus};

use super::*;

fn world(id: &str, name: &str) -> World {
    World {
        id: id.to_owned(),
        name: name.to_owned(),
        game_mode: GameMode::Survival,
        generator: Generator::Normal,
        difficulty: Difficulty::Normal,
        seed: 1,
        created_unix: 0,
        last_played_unix: 0,
    }
}

fn status(state: WorldState, id: &str) -> WorldStatus {
    WorldStatus {
        state,
        world_id: Some(id.to_owned()),
        paused: false,
        error: None,
    }
}

fn loaded(names: &[&str]) -> WorldsMenu {
    let mut menu = WorldsMenu::default();
    assert_eq!(menu.update(Input::Refresh), vec![Effect::List]);
    menu.apply(Event::Listed(
        names
            .iter()
            .enumerate()
            .map(|(i, n)| world(&format!("id{i}"), n))
            .collect(),
    ));
    menu
}

#[test]
fn refresh_selects_first_world_and_keeps_selection_by_id() {
    let mut menu = loaded(&["a", "b", "c"]);
    assert_eq!(menu.selected().map(|w| w.name.as_str()), Some("a"));
    menu.update(Input::Select(2));
    menu.update(Input::Refresh);
    menu.apply(Event::Listed(vec![world("new", "n"), world("id2", "c")]));
    assert_eq!(menu.selected().map(|w| w.id.as_str()), Some("id2"));
}

#[test]
fn empty_list_has_no_selection_and_ignores_actions() {
    let mut menu = loaded(&[]);
    assert!(menu.selected().is_none());
    assert!(menu.update(Input::Play).is_empty());
    assert!(menu.update(Input::RequestDelete).is_empty());
    assert_eq!(menu.screen(), Screen::List);
}

#[test]
fn selection_movement_clamps() {
    let mut menu = loaded(&["a", "b"]);
    menu.update(Input::MoveSelection(5));
    assert_eq!(menu.selected_index(), Some(1));
    menu.update(Input::MoveSelection(-9));
    assert_eq!(menu.selected_index(), Some(0));
}

#[test]
fn create_validates_then_submits_and_lists_new_world_first() {
    let mut menu = loaded(&["a"]);
    menu.update(Input::BeginCreate);
    assert_eq!(menu.screen(), Screen::Create);
    menu.update(Input::SetName("  ".to_owned()));
    assert!(menu.update(Input::SubmitCreate).is_empty());
    assert!(menu.form_error().is_some());
    menu.update(Input::SetName("Fresh".to_owned()));
    menu.update(Input::SetSeed("0".to_owned()));
    menu.update(Input::CycleGameMode);
    let effects = menu.update(Input::SubmitCreate);
    let [Effect::Create(new_world)] = effects.as_slice() else {
        panic!("expected one create effect, got {effects:?}");
    };
    assert_eq!(new_world.name, "Fresh");
    assert_eq!(new_world.seed, Some(0));
    assert_eq!(new_world.game_mode, GameMode::Creative);
    assert!(menu.busy());
    assert!(
        menu.update(Input::SubmitCreate).is_empty(),
        "busy blocks double submit"
    );
    menu.apply(Event::Created(world("fresh", "Fresh")));
    assert_eq!(menu.screen(), Screen::List);
    assert_eq!(menu.selected().map(|w| w.id.as_str()), Some("fresh"));
    assert_eq!(menu.worlds().len(), 2);
    assert!(!menu.busy());
}

#[test]
fn delete_requires_confirmation_and_back_cancels() {
    let mut menu = loaded(&["a", "b"]);
    assert!(
        menu.update(Input::ConfirmDelete).is_empty(),
        "no delete without the confirm screen"
    );
    menu.update(Input::RequestDelete);
    assert_eq!(menu.screen(), Screen::ConfirmDelete);
    menu.update(Input::Back);
    assert_eq!((menu.screen(), menu.worlds().len()), (Screen::List, 2));
    menu.update(Input::RequestDelete);
    assert_eq!(
        menu.update(Input::ConfirmDelete),
        vec![Effect::Delete("id0".to_owned())]
    );
    menu.apply(Event::Deleted("id0".to_owned()));
    assert_eq!(menu.worlds().len(), 1);
    assert_eq!(menu.selected().map(|w| w.id.as_str()), Some("id1"));
    assert_eq!(menu.screen(), Screen::List);
}

#[test]
fn deleting_the_last_world_clears_selection() {
    let mut menu = loaded(&["only"]);
    menu.update(Input::RequestDelete);
    menu.update(Input::ConfirmDelete);
    menu.apply(Event::Deleted("id0".to_owned()));
    assert!(menu.selected().is_none());
}

#[test]
fn rename_prefills_validates_and_applies() {
    let mut menu = loaded(&["Old"]);
    menu.update(Input::BeginRename);
    assert_eq!(menu.rename_text(), "Old");
    menu.update(Input::SetRenameText(String::new()));
    assert!(menu.update(Input::SubmitRename).is_empty());
    assert!(menu.form_error().is_some());
    menu.update(Input::SetRenameText(" New ".to_owned()));
    assert_eq!(
        menu.update(Input::SubmitRename),
        vec![Effect::Rename {
            id: "id0".to_owned(),
            name: "New".to_owned()
        }]
    );
    menu.apply(Event::Renamed(world("id0", "New")));
    assert_eq!(menu.selected().map(|w| w.name.as_str()), Some("New"));
    assert_eq!(menu.screen(), Screen::List);
}

#[test]
fn play_polls_until_running_then_hands_off_once() {
    let mut menu = loaded(&["a"]);
    assert_eq!(
        menu.update(Input::Play),
        vec![Effect::Open("id0".to_owned())]
    );
    assert_eq!(menu.screen(), Screen::Opening);
    assert_eq!(menu.opening_name(), Some("a"));
    assert_eq!(
        menu.apply(Event::Status(status(WorldState::Starting, "id0"))),
        vec![Effect::PollStatus]
    );
    assert!(menu.take_ready().is_none());
    assert!(
        menu.apply(Event::Status(status(WorldState::Running, "id0")))
            .is_empty()
    );
    assert_eq!(menu.take_ready().as_deref(), Some("id0"));
    assert!(menu.take_ready().is_none());
    assert_eq!(menu.screen(), Screen::List);
}

#[test]
fn opening_failure_shows_error_and_clears_core_failure() {
    let mut menu = loaded(&["a"]);
    menu.update(Input::Play);
    let mut failed = status(WorldState::Failed, "id0");
    failed.error = Some("server exited".to_owned());
    assert_eq!(menu.apply(Event::Status(failed)), vec![Effect::Close]);
    assert_eq!(
        (menu.screen(), menu.error()),
        (Screen::Error, Some("server exited"))
    );
    assert!(menu.take_ready().is_none());
    menu.update(Input::Back);
    assert_eq!((menu.screen(), menu.error()), (Screen::List, None));
}

#[test]
fn back_while_opening_closes_and_ignores_late_status() {
    let mut menu = loaded(&["a"]);
    menu.update(Input::Play);
    assert_eq!(menu.update(Input::Back), vec![Effect::Close]);
    assert_eq!(menu.screen(), Screen::List);
    assert!(
        menu.apply(Event::Status(status(WorldState::Running, "id0")))
            .is_empty()
    );
    assert!(
        menu.take_ready().is_none(),
        "a cancelled open must not hand off"
    );
}

#[test]
fn status_for_another_world_is_ignored() {
    let mut menu = loaded(&["a", "b"]);
    menu.update(Input::Play);
    assert!(
        menu.apply(Event::Status(status(WorldState::Running, "id1")))
            .is_empty()
    );
    assert!(menu.take_ready().is_none());
    assert_eq!(menu.screen(), Screen::Opening);
}

#[test]
fn request_failure_surfaces_message_and_unblocks() {
    let mut menu = loaded(&["a"]);
    menu.update(Input::Refresh);
    assert!(menu.busy());
    assert!(
        menu.apply(Event::Failed(
            "Local world service is unavailable".to_owned()
        ))
        .is_empty()
    );
    assert_eq!(menu.screen(), Screen::Error);
    assert!(!menu.busy());
}
