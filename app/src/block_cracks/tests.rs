use client_world::{CommittedUiEvent, WorldStream};
use protocol::{WorldBootstrap, WorldEvent};
use world::{ChunkCollisionRevision, ChunkKey};

use super::*;

fn event(position: [i32; 3], action: BlockCrackAction) -> BlockCrackEvent {
    BlockCrackEvent { position, action }
}

fn start(value: u16) -> BlockCrackAction {
    BlockCrackAction::Start {
        progress_per_tick: value,
    }
}

fn update(value: u16) -> BlockCrackAction {
    BlockCrackAction::UpdateSpeed {
        progress_per_tick: value,
    }
}

fn identity(runtime_id: u32, revision: u64) -> CrackTargetIdentity {
    let mut layers = [None; world::MAX_STORAGE_COUNT];
    layers[0] = Some(runtime_id);
    CrackTargetIdentity {
        runtime_id,
        layers,
        column: ChunkCollisionRevision {
            chunk: ChunkKey::new(0, -1, 0),
            revision,
        },
    }
}

#[test]
fn block_crack_reducer_preserves_values_without_progress_invention() {
    let mut state = BlockCracks::default();
    state.consume(0, event([-1, 64, 0], start(123)));
    state.reconcile_targets(|_| Some(identity(7, 1)));
    assert_eq!(state.active[&[-1, 64, 0]].server_value, 123);
    state.consume(0, event([-1, 64, 0], update(456)));
    assert_eq!(state.active[&[-1, 64, 0]].server_value, 456);
    assert_eq!(state.active[&[-1, 64, 0]].target, Some(identity(7, 1)));
    for _ in 0..100 {
        state.reconcile_targets(|_| Some(identity(7, 1)));
    }
    assert_eq!(state.active[&[-1, 64, 0]].server_value, 456);
    state.consume(0, event([-1, 64, 0], start(789)));
    assert_eq!(state.active[&[-1, 64, 0]].server_value, 789);
    assert_eq!(state.active[&[-1, 64, 0]].target, None);
    state.consume(0, event([-1, 64, 0], BlockCrackAction::Stop));
    assert_eq!(state.status().active, 0);
}

#[test]
fn block_crack_full_capacity_keeps_updates_and_stops_usable() {
    let mut state = BlockCracks::default();
    for position in 0..MAX_ACTIVE_BLOCK_CRACKS {
        state.consume(0, event([i32::try_from(position).unwrap(), 0, 0], start(1)));
    }
    state.consume(0, event([-1, 0, 0], start(1)));
    assert_eq!(state.status().capacity_rejections, 1);
    assert_eq!(state.status().active, MAX_ACTIVE_BLOCK_CRACKS);
    state.consume(0, event([0, 0, 0], update(27)));
    assert_eq!(state.active[&[0, 0, 0]].server_value, 27);
    state.consume(0, event([0, 0, 0], start(30)));
    assert_eq!(state.active[&[0, 0, 0]].server_value, 30);
    state.consume(0, event([0, 0, 0], BlockCrackAction::Stop));
    state.consume(0, event([-1, 0, 0], start(42)));
    assert_eq!(state.status().active, MAX_ACTIVE_BLOCK_CRACKS);
    assert_eq!(state.active[&[-1, 0, 0]].server_value, 42);
    assert_eq!(state.status().capacity_rejections, 1);
}

#[test]
fn block_crack_unsupported_and_orphan_values_are_counted_not_authority() {
    let mut state = BlockCracks::default();
    state.consume(0, event([0; 3], update(1)));
    state.consume(0, event([0; 3], BlockCrackAction::Stop));
    assert_eq!(state.status().active, 0);
    assert_eq!(state.status().orphan_updates, 1);
    state.consume(0, event([0; 3], start(5)));
    state.consume(0, event([0; 3], start(0)));
    state.consume(0, event([0; 3], update(0)));
    assert_eq!(state.active[&[0; 3]].server_value, 5);
    assert_eq!(state.status().unsupported_values, 2);
    state.consume(1, event([0; 3], BlockCrackAction::Stop));
    assert_eq!(state.status().active, 1);
    assert_eq!(state.status().wrong_dimension, 1);
}

#[test]
fn block_crack_target_reconciliation_retires_unknown_and_replaced_layers() {
    let mut state = BlockCracks::default();
    state.consume(0, event([0; 3], start(1)));
    state.reconcile_targets(|_| None);
    assert_eq!(state.status().retired_targets, 1);
    let mut changed_layer = identity(7, 1);
    changed_layer.layers[1] = Some(8);
    for changed in [identity(8, 1), changed_layer] {
        state.consume(0, event([0; 3], start(1)));
        state.reconcile_targets(|_| Some(identity(7, 1)));
        state.reconcile_targets(|_| Some(changed));
        assert_eq!(state.status().active, 0);
    }
    assert_eq!(state.status().retired_targets, 3);
}

#[test]
fn block_crack_exact_cell_survives_unrelated_column_mutation_and_retires_on_air_or_unload() {
    let mut store = world::ChunkStore::new();
    let key = world::SubChunkKey::new(0, -1, 4, -2);
    store.mark_chunk_loaded(key.chunk()).unwrap();
    let air = protocol::SEQUENTIAL_AIR_NETWORK_ID;
    store
        .update_block(key, world::BlockUpdate::new(15, 0, 15, 0, 0), air)
        .unwrap();
    let assets = assets::RuntimeAssets::diagnostic();
    let sample = |store: &world::ChunkStore| {
        sample_target(
            store,
            0,
            [-1, 64, -17],
            assets::NetworkIdMode::Sequential,
            &assets,
        )
    };
    let mut state = BlockCracks::default();
    state.consume(0, event([-1, 64, -17], start(41)));
    state.reconcile_targets(|_| sample(&store));
    let initial = state.active[&[-1, 64, -17]].target.unwrap();
    store
        .update_block(key, world::BlockUpdate::new(1, 0, 1, 0, 0), air)
        .unwrap();
    assert_ne!(initial.column, sample(&store).unwrap().column);
    state.reconcile_targets(|_| sample(&store));
    assert_eq!(state.status().active, 1);
    assert_eq!(state.status().server_value_sum, 41);
    store
        .update_block(key, world::BlockUpdate::new(15, 0, 15, 0, air), air)
        .unwrap();
    state.reconcile_targets(|_| sample(&store));
    assert_eq!(state.status().active, 0);
    store
        .update_block(key, world::BlockUpdate::new(15, 0, 15, 0, 0), air)
        .unwrap();
    state.consume(0, event([-1, 64, -17], start(42)));
    state.reconcile_targets(|_| sample(&store));
    assert_eq!(state.status().active, 1);
    store.evict_chunk(key.chunk());
    state.reconcile_targets(|_| sample(&store));
    assert_eq!(state.status().active, 0);
    assert_eq!(state.status().retired_targets, 2);
}

#[test]
fn block_crack_lifecycle_clears_keys_but_not_the_fifo_watermark() {
    let mut ui = UiRuntime::new(9);
    consume_committed_block_crack(&mut ui, 9, 10, 0, event([0; 3], start(1))).unwrap();
    ui.note_stream_dimension(1);
    assert_eq!(ui.block_cracks_status().active, 0);
    assert!(matches!(
        consume_committed_block_crack(&mut ui, 9, 10, 1, event([0; 3], start(1))),
        Err(UiRuntimeError::StaleBlockCrackSequence { .. })
    ));
    consume_committed_block_crack(&mut ui, 9, 11, 0, event([0; 3], start(1))).unwrap();
    assert_eq!(ui.block_cracks_status().wrong_dimension, 1);
    consume_committed_block_crack(&mut ui, 9, 12, 1, event([0; 3], start(2))).unwrap();
    ui.clear_disconnected_block_cracks();
    assert_eq!(ui.block_cracks_status().active, 0);
    assert!(consume_committed_block_crack(&mut ui, 9, 12, 1, event([0; 3], start(1))).is_err());
    ui.begin_session(10);
    assert_eq!(ui.block_cracks_status(), BlockCrackStatus::default());
    assert!(matches!(
        consume_committed_block_crack(&mut ui, 9, 13, 1, event([0; 3], start(1))),
        Err(UiRuntimeError::WrongSession { .. })
    ));
    consume_committed_block_crack(&mut ui, 10, 1, 1, event([0; 3], start(3))).unwrap();
    assert_eq!(ui.block_cracks_status().active, 1);
}

fn stream() -> WorldStream {
    WorldStream::new(WorldBootstrap {
        local_player_unique_id: 1,
        local_player_runtime_id: 1,
        dimension: 0,
        player_position: [0.0; 3],
        world_spawn_position: [0; 3],
        air_network_id: protocol::SEQUENTIAL_AIR_NETWORK_ID,
        block_network_ids_are_hashes: false,
    })
}

#[test]
fn block_crack_production_stream_batches_consume_beyond_former_history_limit() {
    let mut stream = stream();
    let mut ui = UiRuntime::new(9);
    ui.note_stream_dimension(0);
    for batch in 0..40_u64 {
        for offset in 0..32_u64 {
            stream
                .submit(
                    batch * 32 + offset + 1,
                    WorldEvent::BlockCrack(event([0; 3], BlockCrackAction::Stop)),
                )
                .unwrap();
        }
        for committed in stream.take_committed_ui() {
            let CommittedUiEvent::BlockCrack {
                sequence,
                dimension,
                event,
            } = committed
            else {
                panic!("expected committed crack");
            };
            consume_committed_block_crack(&mut ui, 9, sequence, dimension, event).unwrap();
        }
        reconcile_world_block_cracks(&mut ui, &stream, &assets::RuntimeAssets::diagnostic());
        assert_eq!(ui.block_cracks_status().active, 0);
    }
    assert_eq!(ui.block_cracks_status().consumed, 1_280);
    assert!(stream.take_committed_ui().is_empty());
}

#[test]
fn block_crack_production_world_reconciliation_clears_unloaded_targets() {
    let stream = stream();
    let mut ui = UiRuntime::new(9);
    ui.note_stream_dimension(0);
    consume_committed_block_crack(&mut ui, 9, 1, 0, event([-1, 64, -17], start(1))).unwrap();
    assert_eq!(ui.block_cracks_status().active, 1);
    reconcile_world_block_cracks(&mut ui, &stream, &assets::RuntimeAssets::diagnostic());
    assert_eq!(ui.block_cracks_status().active, 0);
    assert_eq!(ui.block_cracks_status().retired_targets, 1);
}

#[test]
fn block_crack_consumer_is_wired_to_the_production_committed_dispatch() {
    let source = include_str!("../runtime/world.rs");
    let drive = source
        .split_once("pub(crate) fn drive_world_stream(")
        .unwrap()
        .1;
    assert!(drive.contains("} => consume_committed_block_crack("));
    assert!(drive.contains("reconcile_world_block_cracks(&mut ui_runtime, stream, &crack_assets)"));
    assert!(drive.contains("ui_runtime.clear_disconnected_block_cracks()"));
}
