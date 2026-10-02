use std::sync::Arc;

use super::*;
use crate::session_audio::{AudioOutcome, SessionAudio};

fn event(session: u64, sequence: u64) -> SequencedAudioEvent {
    SequencedAudioEvent {
        origin_stream_session_id: session,
        dimension: 0,
        dimension_epoch: 0,
        sequence,
        event: protocol::AudioEvent::Play(protocol::PlayAudioEvent {
            name: Arc::from(TARGET),
            position: [9, -17, 25],
            volume: 1.0,
            pitch: 1.0,
            loop_count: -1,
            server_sound_handle: Some(123456789),
        }),
    }
}

#[test]
fn exact_selector_is_off_by_default_and_rejects_other_values() {
    for value in [None, Some(""), Some("true"), Some("other.sound")] {
        let mut evidence = WireEvidence::new(selected(value));
        evidence.bind(Some(1));
        assert!(evidence.observe(1, &event(1, 1)).is_none());
        assert!(evidence.rows.is_empty());
    }
    assert!(selected(Some(TARGET)));
}

#[test]
fn four_rows_are_bounded_and_session_disconnect_resets_the_budget() {
    let mut evidence = WireEvidence::new(true);
    evidence.bind(Some(1));
    for sequence in 1..=9 {
        assert_eq!(
            evidence.observe(1, &event(1, sequence)).is_some(),
            sequence <= 4
        );
    }
    assert_eq!(evidence.rows.len(), 4);
    evidence.bind(None);
    assert!(evidence.rows.is_empty());
    evidence.bind(Some(2));
    assert!(evidence.observe(2, &event(2, 1)).is_some());
}

#[test]
fn producer_session_and_strict_fifo_are_required_without_rebinding() {
    let mut evidence = WireEvidence::new(true);
    evidence.bind(Some(2));
    assert!(evidence.observe(2, &event(1, 500)).is_none());
    assert!(evidence.observe(1, &event(1, 500)).is_none());
    assert!(evidence.observe(2, &event(2, 3)).is_some());
    for sequence in [3, 2, 1] {
        assert!(evidence.observe(2, &event(2, sequence)).is_none());
    }
    let mut unrelated = event(2, 4);
    let protocol::AudioEvent::Play(play) = &mut unrelated.event else {
        unreachable!()
    };
    play.name = Arc::from("unrelated.test.sound");
    assert!(evidence.observe(2, &unrelated).is_none());
    assert!(evidence.observe(2, &event(2, 4)).is_none());
    assert!(evidence.observe(2, &event(2, 5)).is_some());
    assert_eq!(evidence.rows.len(), 2);
}

#[test]
fn serialization_has_only_fixed_decoded_fields_and_handle_presence() {
    let mut evidence = WireEvidence::new(true);
    evidence.bind(Some(1));
    let row = evidence.observe(1, &event(1, 1)).unwrap();
    let value = serde_json::to_value(row).unwrap();
    let keys: std::collections::BTreeSet<_> = value
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    assert_eq!(
        keys,
        std::collections::BTreeSet::from([
            "schema",
            "authority",
            "origin_stream_session_id",
            "observed_fifo_sequence",
            "position_eighth_blocks",
            "position_blocks",
            "loop_count",
            "gain_bits",
            "pitch_bits",
            "server_sound_handle_present",
        ])
    );
    assert_eq!(
        value["position_eighth_blocks"],
        serde_json::json!([9, -17, 25])
    );
    assert_eq!(
        value["position_blocks"],
        serde_json::json!([1.125, -2.125, 3.125])
    );
    assert_eq!(value["loop_count"], -1);
    assert_eq!(value["gain_bits"], 1.0_f32.to_bits());
    assert_eq!(value["pitch_bits"], 1.0_f32.to_bits());
    assert_eq!(value["server_sound_handle_present"], true);
    let text = serde_json::to_string(row).unwrap();
    for forbidden in [
        "123456789",
        TARGET,
        "account",
        "address",
        "payload",
        "packet_bytes",
    ] {
        assert!(!text.contains(forbidden));
    }
    let mut absent = event(1, 2);
    let protocol::AudioEvent::Play(play) = &mut absent.event else {
        unreachable!()
    };
    play.server_sound_handle = None;
    let row = evidence.observe(1, &absent).unwrap();
    assert!(!row.server_sound_handle_present);
}

fn stream() -> client_world::WorldStream {
    client_world::WorldStream::new(protocol::WorldBootstrap {
        dimension: 0,
        local_player_runtime_id: 1,
        local_player_unique_id: 1,
        player_position: [0.0, 64.0, 0.0],
        world_spawn_position: [0, 64, 0],
        air_network_id: protocol::SEQUENTIAL_AIR_NETWORK_ID,
        block_network_ids_are_hashes: false,
    })
}

fn forwarded(stream: &mut client_world::WorldStream, sequence: u64) -> Vec<SequencedAudioEvent> {
    stream
        .submit(
            sequence,
            protocol::WorldEvent::Audio(event(0, sequence).event),
        )
        .unwrap();
    let mut events = Vec::new();
    crate::runtime::audio::drain_committed_audio(stream, |event| events.push(event));
    events
}

fn app(stream: client_world::WorldStream, enabled: bool) -> bevy::prelude::App {
    let mut app = bevy::prelude::App::new();
    app.add_message::<SequencedAudioEvent>()
        .init_resource::<crate::environment::WorldClock>()
        .insert_resource(crate::runtime::world::ClientWorld {
            stream: Some(stream),
            ..crate::runtime::world::ClientWorld::default()
        })
        .insert_resource(crate::session_audio::SessionAudioCatalog(None))
        .insert_resource(SessionAudio {
            wire_evidence: WireEvidence::new(enabled),
            ..SessionAudio::default()
        })
        .add_systems(
            bevy::prelude::Update,
            crate::session_audio::drain_sequenced_audio_into_session,
        );
    app
}

fn write(app: &mut bevy::prelude::App, events: impl IntoIterator<Item = SequencedAudioEvent>) {
    let mut messages = app
        .world_mut()
        .resource_mut::<bevy::ecs::message::Messages<SequencedAudioEvent>>();
    for event in events {
        messages.write(event);
    }
}

#[test]
fn production_forwarding_rejects_old_buffered_stream_after_fifo_restart() {
    let mut old = stream();
    let old_id = old.actor_session_id();
    let old_events = forwarded(&mut old, 1);
    assert_eq!(old_events[0].origin_stream_session_id, old_id);
    let mut app = app(old, true);
    write(&mut app, old_events);
    app.update();
    assert_eq!(
        app.world()
            .resource::<SessionAudio>()
            .wire_evidence
            .rows
            .len(),
        1
    );
    let old_buffered = {
        let mut world = app
            .world_mut()
            .resource_mut::<crate::runtime::world::ClientWorld>();
        forwarded(world.stream.as_mut().unwrap(), 2)
    };
    let mut replacement = stream();
    let new_id = replacement.actor_session_id();
    assert_ne!(old_id, new_id);
    let new_events = forwarded(&mut replacement, 1);
    app.world_mut()
        .resource_mut::<crate::runtime::world::ClientWorld>()
        .stream = Some(replacement);
    write(&mut app, old_buffered.into_iter().chain(new_events));
    app.update();
    let audio = app.world().resource::<SessionAudio>();
    assert_eq!(audio.wire_evidence.rows.len(), 1);
    assert_eq!(audio.wire_evidence.rows[0].origin_stream_session_id, new_id);
    assert_eq!(audio.wire_evidence.rows[0].observed_fifo_sequence, 1);
    assert_eq!(
        audio.catalog_unavailable_total(),
        3,
        "observer must not filter resolver input"
    );
    let mut events = Vec::new();
    {
        let mut world = app
            .world_mut()
            .resource_mut::<crate::runtime::world::ClientWorld>();
        for sequence in 2..=4 {
            events.extend(forwarded(world.stream.as_mut().unwrap(), sequence));
        }
    }
    write(&mut app, events);
    app.update();
    assert_eq!(
        app.world()
            .resource::<SessionAudio>()
            .wire_evidence
            .rows
            .len(),
        4
    );
    app.world_mut()
        .resource_mut::<crate::runtime::world::ClientWorld>()
        .stream = None;
    write(&mut app, [event(new_id, 5)]);
    app.update();
    assert!(
        app.world()
            .resource::<SessionAudio>()
            .wire_evidence
            .rows
            .is_empty()
    );
    assert!(app.world().resource::<SessionAudio>().is_empty());
}

#[test]
fn production_evidence_precedes_missing_catalog_without_changing_resolution() {
    let mut origin = stream();
    let events = forwarded(&mut origin, 1);
    let mut enabled = app(origin, true);
    let mut disabled = app(stream(), false);
    write(&mut enabled, events.clone());
    write(&mut disabled, events);
    enabled.update();
    disabled.update();
    let enabled = enabled.world().resource::<SessionAudio>();
    let disabled = disabled.world().resource::<SessionAudio>();
    assert_eq!(
        enabled.iter().collect::<Vec<_>>(),
        disabled.iter().collect::<Vec<_>>()
    );
    assert_eq!(
        enabled.catalog_unavailable_total(),
        disabled.catalog_unavailable_total()
    );
    assert!(
        enabled
            .iter()
            .all(|outcome| matches!(outcome, AudioOutcome::Skipped { .. }))
    );
    assert_eq!(enabled.wire_evidence.rows.len(), 1);
    assert!(disabled.wire_evidence.rows.is_empty());
}

#[test]
fn review_replaced_stream_audio_cannot_enter_the_new_session() {
    let mut session = SessionAudio::default();
    session.admit_from_stream(2, 10, 0, vec![event(1, 500), event(2, 1)], None);
    assert_eq!(session.iter().next().unwrap().sequence(), Some(1));
    assert_eq!(session.iter().count(), 1);
}

#[test]
fn review_broken_stdout_cannot_panic_the_audio_session() {
    struct Broken;
    impl std::io::Write for Broken {
        fn write(&mut self, _bytes: &[u8]) -> std::io::Result<usize> {
            Err(std::io::ErrorKind::BrokenPipe.into())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    assert!(std::panic::catch_unwind(|| write_marker(&mut Broken, "{}")).is_ok());
}
