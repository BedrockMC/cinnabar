use super::*;
use protocol::{FormKind, FormRequestEvent, ServerFormModel, TextMenuForm};

fn form(form_id: u32) -> WorldEvent {
    WorldEvent::Ui(UiEvent::Form(FormRequestEvent {
        form_id,
        kind: FormKind::Menu,
        title: Some(Arc::from("Choose")),
        json: Arc::from("{}"),
        model: ServerFormModel::TextMenu(TextMenuForm {
            title: Arc::from("Choose"),
            content: Arc::from("One"),
            buttons: vec![Arc::from("First")].into(),
            omitted_images: 0,
        }),
    }))
}

fn form_stream() -> WorldStream {
    WorldStream::new(WorldBootstrap {
        local_player_unique_id: 1,
        dimension: 0,
        local_player_runtime_id: 1,
        player_position: [0.0; 3],
        world_spawn_position: [0; 3],
        air_network_id: protocol::SEQUENTIAL_AIR_NETWORK_ID,
        block_network_ids_are_hashes: false,
    })
}

fn transition(dimension: i32) -> WorldEvent {
    WorldEvent::ChangeDimension(ChangeDimensionEvent {
        dimension,
        position: [0.0; 3],
    })
}

#[test]
fn form_epoch_uses_committed_transition_sequence_even_when_dimension_returns() {
    let mut stream = form_stream();
    stream.submit(4, form(2)).unwrap();
    stream.submit(3, transition(0)).unwrap();
    stream.submit(2, transition(1)).unwrap();
    assert_eq!(
        stream.form_dimension_epoch(),
        0,
        "uncommitted transitions have no authority"
    );
    stream.submit(1, form(1)).unwrap();
    assert_eq!(stream.current_dimension(), 0);
    assert_eq!(stream.form_dimension_epoch(), 3);
    let entries = stream.take_committed_ui();
    assert!(matches!(
        entries.as_slice(),
        [
            CommittedUiEvent::Form {
                sequence: 1,
                dimension_epoch: 0,
                ..
            },
            CommittedUiEvent::Form {
                sequence: 4,
                dimension_epoch: 3,
                ..
            }
        ]
    ));
    stream.submit(5, transition(0)).unwrap();
    assert_eq!(
        stream.form_dimension_epoch(),
        5,
        "same dimension still starts a new lifetime"
    );
}

#[test]
fn sixty_four_forms_replace_ui_deltas_without_capacity_fanout() {
    let mut stream = form_stream();
    for sequence in 2..=64 {
        stream.submit(sequence, form(sequence as u32)).unwrap();
    }
    stream.submit(1, form(1)).unwrap();
    let entries = stream.take_committed_ui();
    assert_eq!(entries.len(), COMMITTED_UI_CAPACITY);
    for (index, entry) in entries.into_iter().enumerate() {
        assert!(
            matches!(entry, CommittedUiEvent::Form { sequence, dimension_epoch: 0, .. }
            if sequence == index as u64 + 1)
        );
    }
}

#[test]
fn sixty_four_link_and_transition_deltas_preserve_local_mount_capacity() {
    let mut stream = form_stream();
    let event = |sequence: u64| {
        let dimension = ((sequence - 1) / 2 % 2) as i32;
        if sequence.is_multiple_of(2) {
            transition(1 - dimension)
        } else {
            WorldEvent::ActorLink(ActorLinkEvent {
                dimension,
                ridden_unique_id: 90,
                rider_unique_id: 1,
                link_type: ActorLinkType::Rider,
                immediate: false,
                rider_initiated: false,
            })
        }
    };
    for sequence in 2..=64 {
        stream.submit(sequence, event(sequence)).unwrap();
    }
    stream.submit(1, event(1)).unwrap();
    let entries = stream.take_committed_ui();
    assert_eq!(entries.len(), COMMITTED_UI_CAPACITY);
    for (index, entry) in entries.into_iter().enumerate() {
        assert_eq!(
            entry,
            CommittedUiEvent::LocalMount {
                sequence: index as u64 + 1,
                ridden_unique_id: index.is_multiple_of(2).then_some(90),
            }
        );
    }
    assert_eq!(stream.take_committed_controls().len(), 32);
    assert_eq!(stream.form_dimension_epoch(), 64);
}

#[test]
fn ui_and_block_crack_events_publish_fifo_with_committed_dimension() {
    let mut stream = WorldStream::new(WorldBootstrap {
        local_player_unique_id: 1,
        dimension: 2,
        local_player_runtime_id: 1,
        player_position: [0.0; 3],
        world_spawn_position: [0; 3],
        air_network_id: 12_530,
        block_network_ids_are_hashes: false,
    });
    let ui = UiEvent::Hud(HudEvent::Health { health: 17 });
    let crack = BlockCrackEvent {
        position: [-3, 72, 9],
        action: BlockCrackAction::UpdateSpeed {
            progress_per_tick: 2_048,
        },
    };

    stream.submit(1, WorldEvent::Ui(ui.clone())).unwrap();
    stream.submit(2, WorldEvent::BlockCrack(crack)).unwrap();

    assert_eq!(
        stream.take_committed_ui(),
        vec![
            CommittedUiEvent::Ui {
                sequence: 1,
                event: ui,
            },
            CommittedUiEvent::BlockCrack {
                sequence: 2,
                dimension: 2,
                event: crack,
            },
        ]
    );
    assert!(stream.take_committed_ui().is_empty());
}
