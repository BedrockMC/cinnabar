//! The open chat through vanilla `chat.chat_screen`: history, edit box,
//! suggestions, send/back hits and scrolling. Needs the gitignored UI carrier;
//! each test skips when it is absent.

use json_ui::{Draw, DrawNode};

use super::engine_hud_tests::{engine_presentation, engine_presentation_with};
use super::*;
use crate::ui_runtime::presentation::ChatHit;

fn texts(nodes: &[DrawNode]) -> Vec<&str> {
    nodes
        .iter()
        .filter(|node| node.alpha > 0.0)
        .filter_map(|node| match &node.draw {
            Draw::Text { text, .. } => Some(text.as_str()),
            _ => None,
        })
        .collect()
}

fn text_node<'a>(nodes: &'a [DrawNode], wanted: &str) -> Option<&'a DrawNode> {
    nodes
        .iter()
        .find(|node| matches!(&node.draw, Draw::Text { text, .. } if text == wanted))
}

fn chat(runtime: &mut UiRuntime, sequence: u64, message: &str) {
    runtime
        .apply(SequencedUiEvent {
            session_id: 1,
            fifo_sequence: sequence,
            local_millis: 0,
            server_tick: None,
            event: chat_event(message),
        })
        .unwrap();
}

fn suggestions(runtime: &mut UiRuntime, count: usize) {
    runtime.insert_chat_text("/").unwrap();
    let request = runtime.take_chat_autocomplete_request().unwrap();
    runtime
        .apply(SequencedUiEvent {
            session_id: 1,
            fifo_sequence: 100,
            local_millis: 0,
            server_tick: None,
            event: UiEvent::ChatAutocomplete(protocol::ChatAutocompleteEvent {
                enum_name: Arc::from("commands"),
                action: protocol::ChatAutocompleteAction::Replace,
                suggestions: Arc::from(
                    (0..count)
                        .map(|index| Arc::from(format!("/give{index}")))
                        .collect::<Vec<_>>(),
                ),
            }),
        })
        .unwrap();
    assert!(runtime.complete_chat_autocomplete(request));
}

fn build(presentation: &mut UiPresentationRuntime, runtime: &UiRuntime, now: u64) {
    presentation
        .build(runtime, now, [1280, 720], DpiScale::new(1.0).unwrap())
        .unwrap();
}

fn centre(bounds: ui::UiRect) -> UiPoint {
    UiPoint::new(
        (bounds.min().x() + bounds.max().x()) * 0.5,
        (bounds.min().y() + bounds.max().y()) * 0.5,
    )
    .unwrap()
}

#[test]
fn open_chat_draws_the_vanilla_screen_with_history_and_the_edit_box() {
    let Some(mut presentation) = engine_presentation() else {
        return;
    };
    let mut runtime = UiRuntime::new(1);
    chat(&mut runtime, 1, "hello from the server");
    runtime.open_chat();
    runtime.insert_chat_text("typed").unwrap();
    build(&mut presentation, &runtime, 0);
    let nodes = presentation.chat_draw_nodes();
    let shown = texts(nodes);
    assert!(shown.contains(&"hello from the server"), "{shown:?}");
    assert!(shown.contains(&"typed|"), "caret at the end: {shown:?}");
    // The HUD's own chat lines hide while the screen shows the history.
    assert!(
        text_node(presentation.hud_draw_nodes(), "hello from the server")
            .is_none_or(|node| node.alpha <= 0.0)
    );
    build(&mut presentation, &runtime, 600);
    assert!(
        texts(presentation.chat_draw_nodes()).contains(&"typed"),
        "caret blinks off"
    );
}

#[test]
fn suggestions_and_usage_list_above_the_edit_box_and_hit_by_index() {
    let Some(mut presentation) = engine_presentation() else {
        return;
    };
    let mut runtime = UiRuntime::new(1);
    runtime.open_chat();
    suggestions(&mut runtime, 4);
    build(&mut presentation, &runtime, 0);
    let nodes = presentation.chat_draw_nodes();
    let edit = text_node(nodes, "/|").expect("edit box text");
    for index in 0..4 {
        let row = text_node(nodes, &format!("/give{index}")).expect("suggestion row");
        assert!(
            row.dest.y + row.dest.h <= edit.dest.y,
            "rows sit above the edit box"
        );
    }
    let hits = presentation.chat_hits();
    for index in 0..4 {
        let (_, bounds) = hits
            .iter()
            .find(|(hit, _)| *hit == ChatHit::Suggestion(index))
            .expect("suggestion hit");
        assert_eq!(
            presentation.hit_test_chat(centre(*bounds)),
            Some(ChatHit::Suggestion(index))
        );
    }
    for wanted in [ChatHit::Send, ChatHit::Close] {
        let (_, bounds) = hits.iter().find(|(hit, _)| *hit == wanted).expect("button");
        assert_eq!(presentation.hit_test_chat(centre(*bounds)), Some(wanted));
    }
}

#[test]
fn history_opens_on_the_newest_line_and_the_wheel_reveals_older_ones() {
    let Some(mut presentation) = engine_presentation() else {
        return;
    };
    let mut runtime = UiRuntime::new(1);
    for sequence in 1..=80 {
        chat(&mut runtime, sequence, &format!("line {sequence}"));
    }
    runtime.open_chat();
    build(&mut presentation, &runtime, 0);
    let visible = |presentation: &UiPresentationRuntime, wanted: &str| {
        text_node(presentation.chat_draw_nodes(), wanted)
            .is_some_and(|node| node.clip.h > 0.0 && node.dest.y + node.dest.h > node.clip.y)
            && text_node(presentation.chat_draw_nodes(), wanted)
                .is_some_and(|node| node.dest.y < node.clip.y + node.clip.h)
    };
    for wanted in ["line 1", "line 80"] {
        let node = text_node(presentation.chat_draw_nodes(), wanted);
        eprintln!("{wanted}: {:?}", node.map(|node| (&node.dest, &node.clip)));
    }
    assert!(visible(&presentation, "line 80"));
    assert!(!visible(&presentation, "line 1"));
    presentation.scroll_chat(1_000.0, false);
    build(&mut presentation, &runtime, 0);
    assert!(visible(&presentation, "line 1"));
    assert!(!visible(&presentation, "line 80"));
    // A new message jumps back to the newest line.
    chat(&mut runtime, 81, "line 81");
    build(&mut presentation, &runtime, 0);
    assert!(visible(&presentation, "line 81"));
}

#[test]
fn closed_chat_draws_no_screen_and_hits_nothing() {
    let Some(mut presentation) = engine_presentation() else {
        return;
    };
    let mut runtime = UiRuntime::new(1);
    runtime.open_chat();
    build(&mut presentation, &runtime, 0);
    assert!(!presentation.chat_hits().is_empty());
    runtime.close_chat();
    build(&mut presentation, &runtime, 0);
    assert!(presentation.chat_hits().is_empty());
    assert_eq!(
        presentation.hit_test_chat(UiPoint::new(640.0, 700.0).unwrap()),
        None
    );
}

/// Local-only: writes `chat_screen.png` when `CINNABAR_FORM_SNAPSHOT_DIR` is set.
#[test]
fn chat_screen_snapshot() {
    let Some(mut presentation) =
        engine_presentation_with(super::super::forms::pack_harness::font())
    else {
        return;
    };
    let mut runtime = UiRuntime::new(1);
    for sequence in 1..=12 {
        chat(
            &mut runtime,
            sequence,
            &format!("<Steve> message number {sequence}"),
        );
    }
    runtime.open_chat();
    suggestions(&mut runtime, 3);
    let input = presentation
        .build(&runtime, 0, [1280, 720], DpiScale::new(1.0).unwrap())
        .unwrap();
    super::super::forms::snapshot::write(&input, "chat_screen");
}

/// The real carrier supplies the gear, popup and persisted controls without a network session.
#[test]
fn chat_settings_popup_routes_native_controls_and_retains_the_draft() {
    use crate::menu::{
        MenuAction,
        settings_options::{SETTINGS_OPTIONS, SettingsOptions},
    };
    let Some(mut presentation) =
        engine_presentation_with(super::super::forms::pack_harness::font())
    else {
        return;
    };
    let mut runtime = UiRuntime::new(1);
    let lang = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../.local/assets/compiled/vanilla-v1.mcbelang");
    if let Some(lang) = std::fs::read(lang)
        .ok()
        .and_then(|bytes| assets::RuntimeLangCatalog::decode(&bytes).ok())
    {
        runtime.set_lang_catalog(Arc::new(lang));
    }
    chat(&mut runtime, 1, "Visible chat history");
    runtime.open_chat();
    runtime.insert_chat_text("Unsent draft").unwrap();
    let mut options = SettingsOptions::default();
    presentation.set_chat_settings_snapshot((Arc::new(options.clone()), None));
    let input = presentation
        .build(&runtime, 0, [1280, 720], DpiScale::new(1.0).unwrap())
        .unwrap();
    super::super::forms::snapshot::write(&input, "settings-chat-before");
    assert!(
        presentation
            .chat_hits()
            .iter()
            .any(|(hit, _)| *hit == ChatHit::SettingsOpen)
    );
    presentation.set_chat_settings_open(true);
    let input = presentation
        .build(&runtime, 0, [1280, 720], DpiScale::new(1.0).unwrap())
        .unwrap();
    super::super::forms::snapshot::write(&input, "settings-chat-after");
    let hits = presentation.chat_hits();
    assert!(
        hits.iter().any(|(hit, _)| *hit == ChatHit::SettingsClose),
        "{hits:?}"
    );
    let mute = SETTINGS_OPTIONS
        .iter()
        .position(|option| option.name == "hide_chat")
        .unwrap();
    assert!(
        hits.iter()
            .any(|(hit, _)| *hit
                == ChatHit::SettingsAction(MenuAction::SettingsOption(mute as u16, 1))),
        "{hits:?}"
    );
    assert!(!hits.iter().any(|(hit, _)| *hit == ChatHit::Send));
    options.set(mute, 1);
    presentation.set_chat_settings_snapshot((Arc::new(options), None));
    presentation.set_chat_settings_open(false);
    build(&mut presentation, &runtime, 0);
    assert!(!texts(presentation.chat_draw_nodes()).contains(&"Visible chat history"));
    assert_eq!(runtime.chat_editor().as_str(), "Unsent draft");
}
