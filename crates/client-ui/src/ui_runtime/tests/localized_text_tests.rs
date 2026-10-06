//! Localization across text packet surfaces and rawtext arguments.

use super::*;

#[test]
fn localized_text_packets_resolve_marked_keys_for_chat_and_popups() {
    let bytes = assets::encode_lang_catalog(
        [9; 32],
        [10; 32],
        &[assets::LangEntry {
            key: "multiplayer.player.joined".into(),
            value: "%s joined the game".into(),
        }],
    )
    .unwrap();
    for kind in [TextKind::Raw, TextKind::System, TextKind::Popup] {
        let mut player_runtime = player_state::PlayerState::new(1);
        let mut runtime = UiRuntime::new(1);
        runtime.set_lang_catalog(Arc::new(
            assets::RuntimeLangCatalog::decode(&bytes).unwrap(),
        ));
        runtime
            .apply(
                &mut player_runtime,
                envelope(
                    1,
                    1,
                    UiEvent::Text(TextEvent {
                        category: TextCategory::Parameters,
                        kind,
                        needs_translation: true,
                        source: None,
                        message: Arc::from("§e%multiplayer.player.joined"),
                        parameters: Arc::from([Arc::from("Alex")]),
                        xuid: Arc::from(""),
                        platform_chat_id: Arc::from(""),
                        filtered_message: None,
                    }),
                ),
            )
            .unwrap();
        let message = if kind == TextKind::Popup {
            runtime.hud().actionbar().unwrap().text.as_ref()
        } else {
            runtime.chat().messages().back().unwrap().message.as_ref()
        };
        assert_eq!(message, "§eAlex joined the game");
    }
}

#[test]
fn rawtext_game_mode_feedback_localizes_arguments_without_changing_literal_text() {
    let entries = [
        ("createWorldScreen.gameMode.creative", "Creative"),
        ("gameMode.changed", "Your game mode has been updated to %s"),
    ]
    .map(|(key, value)| assets::LangEntry {
        key: key.into(),
        value: value.into(),
    });
    let bytes = assets::encode_lang_catalog([9; 32], [10; 32], &entries).unwrap();
    let mut player_runtime = player_state::PlayerState::new(1);
    let mut runtime = UiRuntime::new(1);
    runtime.set_lang_catalog(Arc::new(
        assets::RuntimeLangCatalog::decode(&bytes).unwrap(),
    ));
    for (sequence, arguments) in [
        r#"["%createWorldScreen.gameMode.creative"]"#,
        r#"{"rawtext":[{"text":"%createWorldScreen.gameMode.creative"}]}"#,
    ]
    .into_iter()
    .enumerate()
    {
        let document = format!(
            r#"{{"rawtext":[{{"translate":"gameMode.changed","with":{arguments}}},{{"text":" | 100% literal %createWorldScreen.gameMode.creative"}}]}}"#
        );
        runtime
            .apply(
                &mut player_runtime,
                envelope(1, sequence as u64 + 1, raw_text_event(&document)),
            )
            .unwrap();
        assert_eq!(
            runtime.chat().messages().back().unwrap().message.as_ref(),
            "Your game mode has been updated to Creative | 100% literal %createWorldScreen.gameMode.creative"
        );
    }
}
