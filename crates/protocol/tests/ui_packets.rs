use bytes::{BufMut, BytesMut};
use protocol::{
    BedrockSession, BossAction, BossColor, ChatAutocompleteAction, FormKind, MAX_CHAT_AUTOCOMPLETE,
    MAX_FORM_JSON_BYTES, MAX_FORM_JSON_DEPTH, MAX_SCORE_ENTRIES_PER_PACKET, MAX_UI_TEXT_BYTES,
    ModalFormResponseSelection, UiEvent, UiPacketError, WorldEvent, decode_batch, into_world_event,
    modal_form_cancel_response, modal_form_submit_response,
};
use valentine::bedrock::codec::BedrockCodec;
use valentine::bedrock::version::v1_26_44::{
    ActorUniqueId, BossEventPacket, CommandOutput, CommandOutputMessage, CommandOutputPacket,
    EnumsBossBarColor, EnumsBossBarOverlay, EnumsBossEventUpdateType, EnumsModalFormCancelReason,
    EnumsPlayStatus, EnumsSetTitlePacketPayloadTitleType, EnumsSoftEnumUpdateType,
    LevelEventPacket, McpePacketData, McpePacketName, ModalFormRequestPacket,
    ModalFormResponsePacket, PlayStatusPacket, SetHealthPacket, SetScorePacket,
    SetScorePacketScoreInfoItem, SetTitlePacket, TextPacket, TextPacketBody,
    TextPacketPayloadMessageOnly, ToastRequestPacket, UpdateSoftEnumPacket, Vec3,
};
use valentine::protocol::wire;

const TEXT_FIXTURE: &[u8] = include_bytes!("../fixtures/text.bin");
const TITLE_FIXTURE: &[u8] = include_bytes!("../fixtures/set_title.bin");
const BOSS_FIXTURE: &[u8] = include_bytes!("../fixtures/boss_event.bin");
const FORM_FIXTURE: &[u8] = include_bytes!("../fixtures/modal_form_request.bin");

/// gophertunnel's `LevelEventStartBlockCracking` / `LevelEventStopBlockCracking`
/// / `LevelEventUpdateBlockCracking` (`minecraft/protocol/packet/level_event.go`).
/// 1.26.40 carries LevelEvent as a raw `event_id` rather than a named enum.
const LEVEL_EVENT_START_BLOCK_CRACKING: i32 = 3600;
const LEVEL_EVENT_UPDATE_BLOCK_CRACKING: i32 = 3602;

fn ui(packet: impl Into<protocol::Packet>) -> Result<UiEvent, UiPacketError> {
    match into_world_event(packet.into(), 0) {
        Ok(Some(WorldEvent::Ui(event))) => Ok(event),
        Ok(other) => panic!("expected UI event, got {other:?}"),
        Err(protocol::WorldPacketError::Ui(error)) => Err(error),
        Err(other) => panic!("unexpected world packet error: {other}"),
    }
}

fn decode_ui_fixture(bytes: &'static [u8]) -> UiEvent {
    let mut packets = decode_batch(bytes.into(), &BedrockSession { shield_item_id: 0 })
        .expect("decode pinned UI fixture");
    assert_eq!(packets.len(), 1);
    match into_world_event(packets.pop().unwrap(), 0).expect("normalize pinned UI fixture") {
        Some(WorldEvent::Ui(event)) => event,
        other => panic!("expected one UI event, got {other:?}"),
    }
}

fn raw_text_packet(message: String) -> TextPacket {
    TextPacket {
        body: TextPacketBody::Raw(TextPacketPayloadMessageOnly { message }),
        ..Default::default()
    }
}

#[test]
fn pinned_gophertunnel_ui_fixtures_normalize_without_vendor_types() {
    assert!(matches!(decode_ui_fixture(TEXT_FIXTURE), UiEvent::Text(_)));
    assert!(matches!(
        decode_ui_fixture(TITLE_FIXTURE),
        UiEvent::Title(_)
    ));
    assert!(matches!(decode_ui_fixture(BOSS_FIXTURE), UiEvent::Boss(_)));
    let UiEvent::Form(form) = decode_ui_fixture(FORM_FIXTURE) else {
        panic!("expected form event")
    };
    assert_eq!(form.form_id, 91);
    assert_eq!(form.kind, FormKind::Menu);
    assert_eq!(form.title.as_deref(), Some("Fixture"));
    assert_eq!(form.json.as_ref(), r#"{"type":"form","title":"Fixture"}"#);
}

#[test]
fn representative_ui_packets_normalize_without_vendor_types() {
    let text = raw_text_packet("§ahello".to_owned());
    let title = SetTitlePacket {
        title_type: EnumsSetTitlePacketPayloadTitleType::Title,
        title_text: "Round one".to_owned(),
        fade_in_time: 5,
        stay_time: 40,
        fade_out_time: 10,
        ..Default::default()
    };
    // `Add` is wire value 0, gophertunnel's `BossEventShow`.
    let boss = BossEventPacket {
        target_actor_id: ActorUniqueId {
            actor_unique_id: 17,
        },
        event_type: EnumsBossEventUpdateType::Add,
        name: "Dragon".to_owned(),
        health_percent: 0.75,
        color: EnumsBossBarColor::RebeccaPurple,
        overlay: EnumsBossBarOverlay::Notched10,
        ..Default::default()
    };
    let form = ModalFormRequestPacket {
        form_id: 91,
        form_uijson: r#"{"type":"form","title":"Pick"}"#.to_owned(),
    };

    assert!(matches!(ui(text).unwrap(), UiEvent::Text(_)));
    assert!(matches!(ui(title).unwrap(), UiEvent::Title(_)));
    let UiEvent::Boss(boss) = ui(boss).unwrap() else {
        panic!("expected boss event")
    };
    assert_eq!(boss.action, BossAction::Show);
    assert_eq!(boss.style.color, BossColor::RebeccaPurple);
    assert_eq!(boss.style.darken_sky, None);
    assert_eq!(boss.style.create_world_fog, None);
    let UiEvent::Form(form) = ui(form).unwrap() else {
        panic!("expected form event")
    };
    assert_eq!(form.form_id, 91);
    assert_eq!(form.kind, FormKind::Menu);
    assert_eq!(form.title.as_deref(), Some("Pick"));
    assert!(matches!(
        ui(SetHealthPacket { health: 19 }).unwrap(),
        UiEvent::Hud(protocol::HudEvent::Health { health: 19 })
    ));
    assert!(matches!(
        ui(PlayStatusPacket {
            status: EnumsPlayStatus::PlayerSpawn,
        })
        .unwrap(),
        UiEvent::Hud(protocol::HudEvent::PlayerStatus(
            protocol::PlayerStatus::PlayerSpawn
        ))
    ));
    let toast = ToastRequestPacket {
        title: "Saved".to_owned(),
        content: "world backed up".to_owned(),
    };
    let UiEvent::Hud(protocol::HudEvent::Toast { title, message }) = ui(toast).unwrap() else {
        panic!("expected toast event")
    };
    assert_eq!(title.as_ref(), "Saved");
    assert_eq!(message.as_ref(), "world backed up");
    // `Replace` is wire value 2, gophertunnel's `SoftEnumActionSet`.
    let autocomplete = UpdateSoftEnumPacket {
        enum_name: "commands".to_owned(),
        values: vec!["give".to_owned(), "gamerule".to_owned()],
        update_type: EnumsSoftEnumUpdateType::Replace,
    };
    let UiEvent::ChatAutocomplete(autocomplete) = ui(autocomplete).unwrap() else {
        panic!("expected autocomplete update")
    };
    assert_eq!(autocomplete.enum_name.as_ref(), "commands");
    assert_eq!(autocomplete.action, ChatAutocompleteAction::Replace);
    assert_eq!(
        autocomplete
            .suggestions
            .iter()
            .map(AsRef::as_ref)
            .collect::<Vec<&str>>(),
        ["give", "gamerule"]
    );
}

#[test]
fn command_output_is_bounded_and_normalized_for_chat_presentation() {
    let packet = CommandOutputPacket {
        output: CommandOutput {
            output_type: "all_output".to_owned(),
            success_count: 1,
            output_messages: vec![CommandOutputMessage {
                message_id: "commands.generic.success".to_owned(),
                successful: true,
                parameters: vec!["sm3".to_owned()],
            }],
            data_set: Some("transfer accepted".to_owned()),
        },
        ..Default::default()
    };
    let UiEvent::CommandOutput(output) = ui(packet).unwrap() else {
        panic!("expected command output event")
    };
    assert_eq!(output.output_type.as_ref(), "all_output");
    assert_eq!(output.success_count, 1);
    assert_eq!(output.messages.len(), 1);
    assert_eq!(
        output.messages[0].message_id.as_ref(),
        "commands.generic.success"
    );
    assert_eq!(output.messages[0].parameters[0].as_ref(), "sm3");
    assert_eq!(output.data.as_deref(), Some("transfer accepted"));
}

#[test]
fn score_entries_carry_their_own_verb() {
    use valentine::bedrock::version::v1_26_44::{ChangeFakePlayerScore, RemoveScore, ScoreboardId};

    // Protocol 2168 moved the add/remove verb into each entry, so one packet may mix
    // removals with changes (gophertunnel `ScoreboardEntry.Marshal`).
    let packet = SetScorePacket {
        score_info: vec![
            SetScorePacketScoreInfoItem::RemoveScore(RemoveScore {
                action: "remove".to_owned(),
                scoreboard_id: ScoreboardId { scoreboard_id: 7 },
                objective_name: Some(Some("kills".to_owned())),
            }),
            SetScorePacketScoreInfoItem::ChangeFakePlayerScore(Box::new(ChangeFakePlayerScore {
                action: "changefakeplayer".to_owned(),
                scoreboard_id: ScoreboardId { scoreboard_id: 8 },
                objective_name: "kills".to_owned(),
                score_value: 12,
                fake_player_name: "Server".to_owned(),
            })),
        ],
    };
    let UiEvent::Score(score) = ui(packet).unwrap() else {
        panic!("expected score event")
    };
    assert_eq!(score.entries.len(), 2);
    assert_eq!(score.entries[0].action, protocol::ScoreAction::Remove);
    assert_eq!(score.entries[0].scoreboard_id, 7);
    assert_eq!(score.entries[0].objective_name.as_ref(), "kills");
    // A removal carries no score or identity on the wire.
    assert_eq!(score.entries[0].score, 0);
    assert_eq!(score.entries[0].identity, protocol::ScoreIdentity::None);
    assert_eq!(score.entries[1].action, protocol::ScoreAction::Change);
    assert_eq!(score.entries[1].scoreboard_id, 8);
    assert_eq!(score.entries[1].score, 12);
    let protocol::ScoreIdentity::FakePlayer(name) = &score.entries[1].identity else {
        panic!("expected a fake-player identity")
    };
    assert_eq!(name.as_ref(), "Server");
}

#[test]
fn remove_score_preserves_both_1_26_44_optional_markers() {
    use valentine::bedrock::version::v1_26_44::{RemoveScore, ScoreboardId};

    let cases = [
        (None, vec![6, b'r', b'e', b'm', b'o', b'v', b'e', 14, 0]),
        (
            Some(None),
            vec![6, b'r', b'e', b'm', b'o', b'v', b'e', 14, 1, 0],
        ),
        (
            Some(Some("obj".to_owned())),
            vec![
                6, b'r', b'e', b'm', b'o', b'v', b'e', 14, 1, 1, 3, b'o', b'b', b'j',
            ],
        ),
    ];

    for (objective_name, expected) in cases {
        let value = RemoveScore {
            action: "remove".to_owned(),
            scoreboard_id: ScoreboardId { scoreboard_id: 7 },
            objective_name,
        };
        let mut encoded = Vec::new();
        value.encode(&mut encoded).expect("encode RemoveScore");
        assert_eq!(encoded, expected);

        let mut input = expected.as_slice();
        let decoded = RemoveScore::decode(&mut input, ()).expect("decode RemoveScore");
        assert_eq!(decoded, value);
        assert!(input.is_empty(), "RemoveScore left trailing bytes");
    }
}

#[test]
fn oversized_text_scores_and_form_json_fail_closed() {
    let text = raw_text_packet("x".repeat(MAX_UI_TEXT_BYTES + 1));
    assert_eq!(
        ui(text).unwrap_err(),
        UiPacketError::TextTooLong {
            bytes: MAX_UI_TEXT_BYTES + 1,
            max: MAX_UI_TEXT_BYTES,
        }
    );

    let scores = SetScorePacket {
        score_info: vec![SetScorePacketScoreInfoItem::default(); MAX_SCORE_ENTRIES_PER_PACKET + 1],
    };
    assert_eq!(
        ui(scores).unwrap_err(),
        UiPacketError::TooManyScores {
            count: MAX_SCORE_ENTRIES_PER_PACKET + 1,
            max: MAX_SCORE_ENTRIES_PER_PACKET,
        }
    );

    let form = ModalFormRequestPacket {
        form_id: 1,
        form_uijson: "x".repeat(MAX_FORM_JSON_BYTES + 1),
    };
    assert_eq!(
        ui(form).unwrap_err(),
        UiPacketError::FormTooLarge {
            bytes: MAX_FORM_JSON_BYTES + 1,
            max: MAX_FORM_JSON_BYTES,
        }
    );
}

#[test]
fn raw_ui_strings_reject_invalid_utf8_before_owned_materialization() {
    let mut payload = BytesMut::new();
    wire::write_var_u32(&mut payload, McpePacketName::ModalFormRequestPacket as u32);
    wire::write_var_u32(&mut payload, 7);
    wire::write_var_u32(&mut payload, 1);
    payload.put_u8(0xff);

    let mut batch = BytesMut::new();
    batch.put_u8(0xfe);
    wire::write_var_u32(&mut batch, payload.len() as u32);
    batch.extend_from_slice(&payload);

    let error = decode_batch(batch.freeze(), &BedrockSession { shield_item_id: 0 })
        .expect_err("invalid UI UTF-8 must fail closed");
    assert!(error.to_string().contains("UTF-8"), "{error}");
}

#[test]
fn raw_score_strings_reject_invalid_utf8_before_owned_materialization() {
    // Protocol 2168 SetScore wire (gophertunnel `SetScore.Marshal` plus
    // `ScoreboardEntry.Marshal`): entry count, then per entry a varuint32
    // variant, the lowercase variant name, the entry id, and the variant body.
    let mut payload = BytesMut::new();
    wire::write_var_u32(&mut payload, McpePacketName::SetScorePacket as u32);
    wire::write_var_u32(&mut payload, 1);
    wire::write_var_u32(&mut payload, 3);
    wire::write_var_u32(&mut payload, "changefakeplayer".len() as u32);
    payload.extend_from_slice(b"changefakeplayer");
    wire::write_var_u64(&mut payload, 2);
    wire::write_var_u32(&mut payload, 1);
    payload.put_u8(0xff);
    payload.put_i32_le(0);
    wire::write_var_u32(&mut payload, 1);
    payload.put_u8(b'a');

    let mut batch = BytesMut::new();
    batch.put_u8(0xfe);
    wire::write_var_u32(&mut batch, payload.len() as u32);
    batch.extend_from_slice(&payload);

    let error = decode_batch(batch.freeze(), &BedrockSession { shield_item_id: 0 })
        .expect_err("invalid score UTF-8 must fail closed");
    assert!(error.to_string().contains("UTF-8"), "{error}");
}

#[test]
fn raw_text_parameter_count_is_bounded_before_parameter_allocation() {
    // Text wire is unchanged in 1.26.40: NeedsTranslation, the category byte,
    // the message type, then the payload for that category.
    let mut payload = BytesMut::new();
    wire::write_var_u32(&mut payload, McpePacketName::TextPacket as u32);
    payload.put_u8(0);
    payload.put_u8(2);
    payload.put_u8(2);
    wire::write_var_u32(&mut payload, 1);
    payload.put_u8(b'x');
    wire::write_var_u32(&mut payload, (protocol::MAX_CHAT_PARAMETERS + 1) as u32);

    let mut batch = BytesMut::new();
    batch.put_u8(0xfe);
    wire::write_var_u32(&mut batch, payload.len() as u32);
    batch.extend_from_slice(&payload);

    let error = decode_batch(batch.freeze(), &BedrockSession { shield_item_id: 0 })
        .expect_err("oversized text parameter count must fail before allocation");
    assert!(error.to_string().contains("parameters"), "{error}");
}

#[test]
fn raw_soft_enum_count_is_bounded_before_suggestion_allocation() {
    let mut payload = BytesMut::new();
    wire::write_var_u32(&mut payload, McpePacketName::UpdateSoftEnumPacket as u32);
    wire::write_var_u32(&mut payload, 8);
    payload.extend_from_slice(b"commands");
    wire::write_var_u32(&mut payload, (MAX_CHAT_AUTOCOMPLETE + 1) as u32);

    let mut batch = BytesMut::new();
    batch.put_u8(0xfe);
    wire::write_var_u32(&mut batch, payload.len() as u32);
    batch.extend_from_slice(&payload);

    let error = decode_batch(batch.freeze(), &BedrockSession { shield_item_id: 0 })
        .expect_err("oversized soft enum count must fail before allocation");
    assert!(error.to_string().contains("suggestions"), "{error}");
}

#[test]
fn block_crack_events_preserve_server_progress_rate_without_inventing_stage_or_actor() {
    let start = LevelEventPacket {
        event_id: LEVEL_EVENT_START_BLOCK_CRACKING,
        position: Vec3 {
            x: 1.0,
            y: 64.0,
            z: -2.0,
        },
        data: 6_553,
    };
    let Some(WorldEvent::BlockCrack(start)) = into_world_event(start.into(), 0).unwrap() else {
        panic!("expected block crack start")
    };
    assert_eq!(start.position, [1, 64, -2]);
    assert_eq!(
        start.action,
        protocol::BlockCrackAction::Start {
            progress_per_tick: 6_553
        }
    );

    let fractional = LevelEventPacket {
        event_id: LEVEL_EVENT_UPDATE_BLOCK_CRACKING,
        position: Vec3 {
            x: 1.5,
            y: 64.0,
            z: -2.0,
        },
        data: 1,
    };
    assert!(matches!(
        into_world_event(fractional.into(), 0),
        Err(protocol::WorldPacketError::Ui(
            UiPacketError::InvalidBlockCrackPosition { field: "x", .. }
        ))
    ));

    let overflowing = LevelEventPacket {
        event_id: LEVEL_EVENT_START_BLOCK_CRACKING,
        position: Vec3 {
            x: 2_147_483_648.0,
            y: 64.0,
            z: -2.0,
        },
        data: 1,
    };
    assert!(matches!(
        into_world_event(overflowing.into(), 0),
        Err(protocol::WorldPacketError::Ui(
            UiPacketError::InvalidBlockCrackPosition { field: "x", .. }
        ))
    ));
}

fn form_event(json: &str) -> Result<protocol::FormRequestEvent, UiPacketError> {
    let packet = ModalFormRequestPacket {
        form_id: 3,
        form_uijson: json.to_owned(),
    };
    match ui(packet) {
        Ok(UiEvent::Form(form)) => Ok(form),
        Ok(other) => panic!("expected form event, got {other:?}"),
        Err(error) => Err(error),
    }
}

#[test]
fn text_button_form_model_preserves_unicode_and_wire_indices() {
    let event = form_event(r#"{"type":"form","title":"Choose 世界","content":"Pick one\nα β","buttons":[{"text":"First ✓"},{"text":"第二"}]}"#).unwrap();
    let protocol::ServerFormModel::TextMenu(menu) = event.model else {
        panic!("expected supported text-only menu")
    };
    assert_eq!(menu.title.as_ref(), "Choose 世界");
    assert_eq!(menu.content.as_ref(), "Pick one\nα β");
    assert_eq!(menu.buttons.len(), 2);
    assert_eq!(menu.buttons[1].as_ref(), "第二");
}

#[test]
fn element_button_form_model_preserves_unicode_and_wire_indices() {
    let event = form_event(r#"{"type":"form","title":"Menu 世界","content":"Select α β","elements":[{"type":"button","text":"First ✓","image":null},{"type":"button","text":"第二","image":null}]}"#).unwrap();
    let protocol::ServerFormModel::TextMenu(menu) = event.model else {
        panic!("element buttons must normalize as a supported text menu")
    };
    assert_eq!(menu.title.as_ref(), "Menu 世界");
    assert_eq!(menu.content.as_ref(), "Select α β");
    assert_eq!(
        menu.buttons
            .iter()
            .map(|label| label.as_ref())
            .collect::<Vec<_>>(),
        ["First ✓", "第二"]
    );
    assert_eq!(menu.omitted_images, 0);
}

#[test]
fn element_button_forms_reject_ambiguous_or_unsupported_controls_without_renumbering() {
    for json in [
        r#"{"type":"form","buttons":[],"elements":[]}"#,
        r#"{"type":"form","buttons":null,"elements":[]}"#,
        r#"{"type":"form","elements":null}"#,
        r#"{"type":"form","elements":{}}"#,
        r#"{"type":"form","elements":["button"]}"#,
        r#"{"type":"form","elements":[{"text":"A","image":null}]}"#,
        r#"{"type":"form","elements":[{"type":7,"text":"A","image":null}]}"#,
        r#"{"type":"form","elements":[{"type":"button","text":"A"}]}"#,
        r#"{"type":"form","elements":[{"type":"button","image":null}]}"#,
        r#"{"type":"form","elements":[{"type":"button","text":7,"image":null}]}"#,
        r#"{"type":"form","elements":[{"type":"button","text":"A","image":{"type":"path","data":"ignored"}}]}"#,
        r#"{"type":"form","elements":[{"type":"button","text":"A","image":null,"unknown":true}]}"#,
        r#"{"type":"form","elements":[{"type":"button","text":"A","image":null},{"type":"label","text":"B","image":null},{"type":"button","text":"C","image":null}]}"#,
    ] {
        assert_eq!(
            form_event(json).unwrap().model,
            protocol::ServerFormModel::Unsupported(protocol::UnsupportedForm::Controls)
        );
    }
}

#[test]
fn element_button_forms_keep_the_existing_count_and_text_limits() {
    let buttons = std::iter::repeat_n(
        r#"{"type":"button","text":"x","image":null}"#,
        protocol::MAX_FORM_BUTTONS,
    )
    .collect::<Vec<_>>()
    .join(",");
    let event = form_event(&format!(r#"{{"type":"form","elements":[{buttons}]}}"#)).unwrap();
    let protocol::ServerFormModel::TextMenu(menu) = event.model else {
        panic!("bounded element menu")
    };
    assert_eq!(menu.buttons.len(), protocol::MAX_FORM_BUTTONS);
    let overflow = format!(
        r#"{{"type":"form","elements":[{buttons},{{"type":"button","text":"x","image":null}}]}}"#
    );
    let long = "x".repeat(MAX_UI_TEXT_BYTES + 1);
    for json in [
        overflow,
        format!(
            r#"{{"type":"form","elements":[{{"type":"button","text":"{long}","image":null}}]}}"#
        ),
    ] {
        assert_eq!(
            form_event(&json).unwrap().model,
            protocol::ServerFormModel::Unsupported(protocol::UnsupportedForm::Limit)
        );
    }
}

#[test]
fn pinned_text_menu_and_response_fixtures_match_exact_wire_payloads() {
    let UiEvent::Form(form) =
        decode_ui_fixture(include_bytes!("../fixtures/modal_form_text_menu.bin"))
    else {
        panic!("expected form")
    };
    let protocol::ServerFormModel::TextMenu(menu) = form.model else {
        panic!("expected text-only menu")
    };
    assert_eq!(form.form_id, 92);
    assert_eq!(menu.buttons[1].as_ref(), "第二");
    for (fixture, mut expected) in [
        (
            include_bytes!("../fixtures/modal_form_response_button.bin").as_slice(),
            modal_form_submit_response(92, ModalFormResponseSelection::ButtonIndex(1)),
        ),
        (
            include_bytes!("../fixtures/modal_form_response_closed.bin").as_slice(),
            modal_form_cancel_response(92),
        ),
        (
            include_bytes!("../fixtures/modal_form_response_busy.bin").as_slice(),
            protocol::modal_form_busy_response(92),
        ),
    ] {
        expected.header.from_subclient = 1;
        expected.header.to_subclient = 2;
        assert_eq!(
            protocol::encode(&expected, &BedrockSession { shield_item_id: 0 })
                .unwrap()
                .as_ref(),
            fixture
        );
        let decoded = decode_batch(
            fixture.to_vec().into(),
            &BedrockSession { shield_item_id: 0 },
        )
        .unwrap();
        assert_eq!(decoded, vec![expected]);
    }
}

#[test]
fn unsupported_form_controls_are_nonfatal_and_never_fake_text_buttons() {
    for json in [
        r#"{"type":"modal","title":"Question"}"#,
        r#"{"type":"custom_form","content":[{"type":"toggle"}]}"#,
        r#"{"type":"form","buttons":[{"text":"Icon","image":{"type":"unknown","data":"ignored"}}]}"#,
        r#"{"type":"form","buttons":[{"text":"Icon","image":{"type":"url","data":5}}]}"#,
        r#"{"type":"form","buttons":[{"text":"Icon","image":null}]}"#,
        r#"{"type":"form","buttons":[{"text":"Icon","image":{"type":"path","data":"ignored","extra":true}}]}"#,
        r#"{"type":"form","buttons":[{"text":{"rawtext":[{"text":"Rich"}]}}]}"#,
    ] {
        assert!(matches!(
            form_event(json).unwrap().model,
            protocol::ServerFormModel::Unsupported(_)
        ));
    }
}

#[test]
fn valid_button_images_are_omitted_without_retaining_uri_or_changing_indexes() {
    let json = serde_json::json!({
        "type": "form", "content": "x".repeat(MAX_UI_TEXT_BYTES),
        "buttons": [
            {"text": "First", "image": {"type": "url", "data": "https://example.invalid/private-decoration"}},
            {"text": "第二", "image": {"type": "path", "data": "textures/private-decoration"}},
        ],
    }).to_string();
    let event = form_event(&json).unwrap();
    assert!(!format!("{:?}", event.model).contains("private-decoration"));
    let protocol::ServerFormModel::TextMenu(menu) = event.model else {
        panic!("valid text buttons must remain playable")
    };
    assert_eq!(menu.omitted_images, 2);
    assert_eq!(menu.content.len(), MAX_UI_TEXT_BYTES);
    assert_eq!(
        menu.buttons
            .iter()
            .map(|text| text.as_ref())
            .collect::<Vec<_>>(),
        ["First", "第二"]
    );
    let response =
        modal_form_submit_response(event.form_id, ModalFormResponseSelection::ButtonIndex(1));
    let McpePacketData::ModalFormResponsePacket(response) = response.data else {
        panic!("response packet")
    };
    assert_eq!(response.json_response.as_deref(), Some("1"));
}

#[test]
fn text_button_form_model_refuses_count_and_string_overflow_without_truncating_indices() {
    let buttons = std::iter::repeat_n(r#"{"text":"x"}"#, protocol::MAX_FORM_BUTTONS + 1)
        .collect::<Vec<_>>()
        .join(",");
    let event = form_event(&format!(r#"{{"type":"form","buttons":[{buttons}]}}"#)).unwrap();
    assert!(matches!(
        event.model,
        protocol::ServerFormModel::Unsupported(_)
    ));
    let text = "x".repeat(MAX_UI_TEXT_BYTES + 1);
    for json in [
        format!(r#"{{"type":"form","content":"{text}","buttons":[]}}"#),
        format!(r#"{{"type":"form","buttons":[{{"text":"{text}"}}]}}"#),
        format!(
            r#"{{"type":"form","buttons":[{{"text":"Icon","image":{{"type":"url","data":"{text}"}}}}]}}"#
        ),
    ] {
        assert!(matches!(
            form_event(&json).unwrap().model,
            protocol::ServerFormModel::Unsupported(_)
        ));
    }
}

#[test]
fn server_form_families_classify_from_the_type_member() {
    let modal = form_event(r#"{"type":"modal","title":"Yes?","content":"Pick one"}"#).unwrap();
    assert_eq!(modal.kind, FormKind::Modal);
    assert_eq!(modal.title.as_deref(), Some("Yes?"));

    let custom = form_event(
        r#"{"type":"custom_form","title":"Settings","content":[{"type":"toggle","text":"On"}]}"#,
    )
    .unwrap();
    assert_eq!(custom.kind, FormKind::Custom);

    // A missing, non-string, or unrecognized type member keeps the raw text
    // without inventing a family.
    assert_eq!(
        form_event(r#"{"title":"T"}"#).unwrap().kind,
        FormKind::Unknown
    );
    assert_eq!(
        form_event(r#"{"type":7,"title":"T"}"#).unwrap().kind,
        FormKind::Unknown
    );
    assert_eq!(
        form_event(r#"{"type":"slider_form"}"#).unwrap().kind,
        FormKind::Unknown
    );

    // Titles may arrive as non-string rawtext components; the family and the
    // raw text still survive with no title metadata.
    let component_title =
        form_event(r#"{"type":"form","title":{"rawtext":[{"text":"Hi"}]}}"#).unwrap();
    assert_eq!(component_title.kind, FormKind::Menu);
    assert_eq!(component_title.title, None);
}

#[test]
fn server_form_metadata_survives_escaped_and_nested_payloads() {
    let escaped = form_event(r#"{"type":"modal","title":"Line\nQuote\"End"}"#).unwrap();
    assert_eq!(escaped.title.as_deref(), Some("Line\nQuote\"End"));

    // Members beyond the metadata are skipped without interpretation.
    let deep = r#"{"type":"custom_form","title":"C","elements":[{"a":[1,2,{"b":"x"}]}]}"#;
    let event = form_event(deep).unwrap();
    assert_eq!(event.kind, FormKind::Custom);
}

#[test]
fn oversized_or_malformed_form_json_is_a_semantic_error_not_wire_fault() {
    for (json, expected) in [
        (r#"{"type":"modal""#, UiPacketError::InvalidFormJson),
        (r#"[1,2,3]"#, UiPacketError::InvalidFormJson),
        (r#"{} trailing"#, UiPacketError::InvalidFormJson),
        (r#"{"type" "modal"}"#, UiPacketError::InvalidFormJson),
    ] {
        assert_eq!(form_event(json).unwrap_err(), expected);
    }
}

#[test]
fn form_json_nesting_is_bounded_before_any_parse() {
    let mut json = String::new();
    for _ in 0..=MAX_FORM_JSON_DEPTH {
        json.push('[');
    }
    for _ in 0..=MAX_FORM_JSON_DEPTH {
        json.push(']');
    }
    assert_eq!(
        form_event(&json).unwrap_err(),
        UiPacketError::FormJsonDepthExceeded {
            depth: MAX_FORM_JSON_DEPTH + 1,
            max: MAX_FORM_JSON_DEPTH,
        }
    );
}

#[test]
fn oversized_form_titles_fail_the_shared_ui_text_budget() {
    let title = "x".repeat(MAX_UI_TEXT_BYTES + 1);
    let json = format!(r#"{{"type":"form","title":"{title}"}}"#);
    assert_eq!(
        form_event(&json).unwrap_err(),
        UiPacketError::TextTooLong {
            bytes: MAX_UI_TEXT_BYTES + 1,
            max: MAX_UI_TEXT_BYTES,
        }
    );
}

#[test]
fn modal_form_responses_encode_exact_submit_and_cancel_markers() {
    let session = BedrockSession { shield_item_id: 0 };

    // Submitting button 2 of form 7: batch header, length, packet id 101, then
    // id, present(1), len 1, '2', cancel absent(0).
    let submit = modal_form_submit_response(7, ModalFormResponseSelection::ButtonIndex(2));
    assert_eq!(
        protocol::encode(&submit, &session).unwrap().as_ref(),
        &[0xfe, 0x06, 101, 0x07, 0x01, 0x01, b'2', 0x00]
    );
    let direct = ModalFormResponsePacket {
        form_id: 7,
        json_response: Some("2".to_owned()),
        form_cancel_reason: None,
    };
    assert_eq!(
        protocol::encode(&submit, &session).unwrap(),
        protocol::encode(&direct.into(), &session).unwrap()
    );

    // Dismissing form 7: id, response absent(0), cancel present(1) UserClosed(0).
    let cancel = modal_form_cancel_response(7);
    assert_eq!(
        protocol::encode(&cancel, &session).unwrap().as_ref(),
        &[0xfe, 0x05, 101, 0x07, 0x00, 0x01, 0x00]
    );
    let direct_cancel = ModalFormResponsePacket {
        form_id: 7,
        json_response: None,
        form_cancel_reason: Some(EnumsModalFormCancelReason::UserClosed),
    };
    assert_eq!(
        protocol::encode(&cancel, &session).unwrap(),
        protocol::encode(&direct_cancel.into(), &session).unwrap()
    );

    // Both directions round-trip through the pinned codec unchanged.
    let mut encoded = Vec::new();
    BedrockCodec::encode(
        &ModalFormResponsePacket {
            form_id: 7,
            json_response: Some("2".to_owned()),
            form_cancel_reason: None,
        },
        &mut encoded,
    )
    .unwrap();
    let mut payload = BytesMut::new();
    wire::write_var_u32(&mut payload, McpePacketName::ModalFormResponsePacket as u32);
    payload.extend_from_slice(&encoded);
    let mut frame = BytesMut::new();
    frame.put_u8(0xfe);
    wire::write_var_u32(&mut frame, payload.len() as u32);
    frame.extend_from_slice(&payload);
    let mut packets = decode_batch(frame.freeze(), &BedrockSession { shield_item_id: 0 }).unwrap();
    assert_eq!(packets.len(), 1);
    assert!(matches!(
        packets.pop().unwrap().data,
        McpePacketData::ModalFormResponsePacket(ModalFormResponsePacket {
            form_id: 7,
            json_response: Some(response),
            form_cancel_reason: None,
        }) if response == "2"
    ));
}
