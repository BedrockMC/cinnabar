use protocol::world_control::{
    Backend, Difficulty, GameMode, Generator, Prefs, UnavailableReason, World, WorldState,
    WorldStatus,
};

use super::super::prompt::{DOCKER_URL, PromptButton, PromptKind};
use super::*;

fn world(id: &str, name: &str) -> World {
    World {
        id: id.to_owned(),
        name: name.to_owned(),
        game_mode: GameMode::Survival,
        generator: Generator::Normal,
        difficulty: Difficulty::Normal,
        backend: Backend::Dragonfly,
        seed: 1,
        created_unix: 0,
        last_played_unix: 0,
    }
}

fn status(state: WorldState, id: &str) -> WorldStatus {
    WorldStatus {
        state,
        world_id: Some(id.to_owned()),
        backend: None,
        paused: false,
        pause_supported: true,
        error: None,
        setup: None,
        backend_unavailable_reason: None,
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

fn with_reason(reason: UnavailableReason) -> WorldStatus {
    let mut status = status(WorldState::Idle, "");
    status.world_id = None;
    status.backend_unavailable_reason = Some(reason);
    status
}

fn docker_menu(reason: UnavailableReason, names: &[&str]) -> WorldsMenu {
    let mut menu = loaded(names);
    menu.apply(Event::Prefs(Prefs::default(), with_reason(reason)));
    menu
}

#[test]
fn no_backend_reason_never_shows_the_docker_modal() {
    let mut menu = loaded(&["a"]);
    menu.apply(Event::Prefs(Prefs::default(), status(WorldState::Idle, "")));
    menu.update(Input::BeginCreate);
    assert_eq!(menu.screen(), Screen::Create);
}

#[test]
fn docker_missing_gates_create_until_play_anyway() {
    let mut menu = docker_menu(UnavailableReason::DockerMissing, &[]);
    assert!(menu.update(Input::BeginCreate).is_empty());
    assert_eq!(menu.screen(), Screen::BackendPrompt);
    assert_eq!(menu.prompt(), Some(PromptKind::DockerMissing));
    assert!(
        menu.update(Input::Prompt(PromptButton::PlayAnyway))
            .is_empty()
    );
    assert_eq!(menu.screen(), Screen::Create);
    menu.update(Input::Back);
    menu.update(Input::BeginCreate);
    assert_eq!(
        menu.screen(),
        Screen::Create,
        "acknowledged for the session"
    );
}

#[test]
fn docker_missing_get_docker_opens_the_site_and_stays() {
    let mut menu = docker_menu(UnavailableReason::DockerMissing, &[]);
    menu.update(Input::BeginCreate);
    assert_eq!(
        menu.update(Input::Prompt(PromptButton::GetDocker)),
        vec![Effect::OpenUrl(DOCKER_URL)]
    );
    assert_eq!(menu.screen(), Screen::BackendPrompt);
}

#[test]
fn dont_show_again_persists_and_continues() {
    let mut menu = docker_menu(UnavailableReason::DockerMissing, &["a"]);
    menu.update(Input::BeginCreate);
    let effects = menu.update(Input::Prompt(PromptButton::DontShowAgain));
    assert_eq!(
        effects,
        vec![Effect::SetPrefs {
            dismiss_docker_prompt: true,
            redetect: false
        }]
    );
    assert_eq!(menu.screen(), Screen::Create);
    // A dismissed docker_missing prompt stays hidden in a fresh session.
    let mut fresh = loaded(&["a"]);
    fresh.apply(Event::Prefs(
        Prefs {
            docker_prompt_dismissed: true,
        },
        with_reason(UnavailableReason::DockerMissing),
    ));
    fresh.update(Input::BeginCreate);
    assert_eq!(fresh.screen(), Screen::Create);
}

#[test]
fn dismissal_does_not_hide_the_docker_not_running_prompt() {
    let mut menu = loaded(&[]);
    menu.apply(Event::Prefs(
        Prefs {
            docker_prompt_dismissed: true,
        },
        with_reason(UnavailableReason::DockerNotRunning),
    ));
    menu.update(Input::BeginCreate);
    assert_eq!(menu.prompt(), Some(PromptKind::DockerNotRunning));
}

#[test]
fn retry_redetects_and_continues_once_docker_is_up() {
    let mut menu = docker_menu(UnavailableReason::DockerNotRunning, &[]);
    menu.update(Input::BeginCreate);
    assert_eq!(
        menu.update(Input::Prompt(PromptButton::Retry)),
        vec![Effect::SetPrefs {
            dismiss_docker_prompt: false,
            redetect: true
        }]
    );
    assert!(menu.busy());
    menu.apply(Event::Prefs(
        Prefs::default(),
        with_reason(UnavailableReason::DockerNotRunning),
    ));
    assert_eq!((menu.screen(), menu.busy()), (Screen::BackendPrompt, false));
    menu.update(Input::Prompt(PromptButton::Retry));
    menu.apply(Event::Prefs(Prefs::default(), status(WorldState::Idle, "")));
    assert_eq!(menu.screen(), Screen::Create);
}

#[test]
fn playing_a_dragonfly_world_skips_the_modal_but_a_bds_world_gets_it() {
    let mut menu = docker_menu(UnavailableReason::DockerNotRunning, &["a"]);
    assert_eq!(
        menu.update(Input::Play),
        vec![Effect::Open("id0".to_owned())]
    );
    menu.update(Input::Back);
    let mut bds = world("id0", "a");
    bds.backend = Backend::Bds;
    menu.apply(Event::Listed(vec![bds]));
    assert!(menu.update(Input::Play).is_empty());
    assert_eq!(menu.screen(), Screen::BackendPrompt);
    assert_eq!(
        menu.update(Input::Prompt(PromptButton::PlayAnyway)),
        vec![Effect::Open("id0".to_owned())]
    );
}

#[test]
fn eula_required_prompts_then_reopens_the_same_world_after_acceptance() {
    let mut menu = loaded(&["a"]);
    menu.update(Input::Play);
    assert!(menu.apply(Event::EulaRequired).is_empty());
    assert_eq!(menu.screen(), Screen::Eula);
    assert!(menu.update(Input::Back).is_empty());
    assert_eq!(menu.screen(), Screen::List);

    menu.update(Input::Play);
    menu.apply(Event::EulaRequired);
    assert_eq!(menu.update(Input::AcceptEula), vec![Effect::AcceptEula]);
    assert!(menu.busy());
    assert_eq!(
        menu.apply(Event::EulaAccepted),
        vec![Effect::Open("id0".to_owned())]
    );
    assert_eq!(
        (menu.screen(), menu.opening_name()),
        (Screen::Opening, Some("a"))
    );
}

#[test]
fn accept_eula_outside_the_eula_screen_does_nothing() {
    let mut menu = loaded(&["a"]);
    assert!(menu.update(Input::AcceptEula).is_empty());
    assert!(!menu.busy());
}

#[test]
fn backend_label_follows_the_reported_runtime() {
    let mut menu = loaded(&[]);
    assert_eq!(menu.active_backend_label(), "Basic server");
    let mut idle = status(WorldState::Idle, "");
    idle.setup = Some(protocol::world_control::Setup {
        state: protocol::world_control::SetupState::Ready,
        version: None,
        bytes_done: 0,
        bytes_total: 0,
        eula_accepted: true,
        error: None,
        runtime: "container".to_owned(),
        reason: None,
    });
    menu.apply(Event::Prefs(Prefs::default(), idle));
    assert_eq!(
        menu.active_backend_label(),
        "Bedrock Dedicated Server (Docker)"
    );
}

#[test]
fn create_defaults_to_superflat_only_where_the_dedicated_server_cannot_run() {
    for (state, generator) in [
        (
            protocol::world_control::SetupState::Ready,
            Generator::Normal,
        ),
        (
            protocol::world_control::SetupState::Unsupported,
            Generator::Flat,
        ),
    ] {
        let mut menu = loaded(&[]);
        let mut idle = status(WorldState::Idle, "");
        idle.setup = Some(protocol::world_control::Setup {
            state,
            version: None,
            bytes_done: 0,
            bytes_total: 0,
            eula_accepted: false,
            error: None,
            runtime: String::new(),
            reason: None,
        });
        menu.apply(Event::Prefs(Prefs::default(), idle));
        menu.update(Input::BeginCreate);
        assert_eq!(menu.create_form().generator, generator, "{state:?}");
    }
}
