use protocol::{
    BlockActionKind::{
        AbortDestroy, ContinueDestroy, CrackBlock, PredictDestroy, StartDestroy, StopDestroy,
    },
    NetworkItemStack, PlayerInputMode, VerifiedNetworkItemStack,
};
use sim::{DestroyConditions, HeldTool};

use super::{
    BlockBreakingAuthority::{Client, Server},
    *,
};
use crate::movement::{
    MovementSource, MovementTicker, PhysicsMovementSample, PhysicsTickEvidenceContext,
    ProcessedMovementState, flush_player_auth_inputs,
};

fn target(position: [i32; 3], block: &str, tool: Option<&str>) -> DestroyTarget {
    DestroyTarget {
        position,
        face: 1,
        runtime_id: 9,
        relative_hit: [0.5, 1.0, 0.5],
        block: sim::block_destroy_info(block),
        conditions: DestroyConditions {
            tool: tool.and_then(HeldTool::from_identifier),
            ..DestroyConditions::default()
        },
        selection: FrozenMiningSelection {
            slot: 2,
            item: VerifiedNetworkItemStack::try_new(
                NetworkItemStack::empty(),
                NetworkItemStack::empty().nbt_digest,
            )
            .unwrap(),
        },
    }
}

fn kinds(payload: &SurvivalTickPayload) -> Vec<(protocol::BlockActionKind, [i32; 3], u8)> {
    payload
        .actions
        .iter()
        .map(|action| (action.kind, action.position, action.face))
        .collect()
}

fn held(
    machine: &mut DestroyMachine,
    target: &DestroyTarget,
    authority: BlockBreakingAuthority,
) -> SurvivalTickPayload {
    machine.step(DestroyInput::Held(Some(target)), true, authority)
}

#[test]
fn server_authority_is_silent_while_cracking_and_completes_with_continue_then_predict() {
    let dirt = target([1, 2, 3], "minecraft:dirt", None);
    let rate = dirt.rate(true).unwrap();
    let mut machine = DestroyMachine::default();
    assert_eq!(
        kinds(&held(&mut machine, &dirt, Server)),
        [(StartDestroy, [1, 2, 3], 1)]
    );
    let mut progress = 0.0_f32;
    loop {
        progress += rate;
        let payload = held(&mut machine, &dirt, Server);
        if progress >= 1.0 {
            assert_eq!(
                kinds(&payload),
                [
                    (ContinueDestroy, [1, 2, 3], 1),
                    (PredictDestroy, [1, 2, 3], 1)
                ]
            );
            assert_eq!(payload.destroy, None);
            break;
        }
        assert!(
            payload.is_empty(),
            "no crack actions under server authority"
        );
    }
    for _ in 0..DESTROY_DELAY_TICKS {
        assert!(
            held(
                &mut machine,
                &target([1, 1, 3], "minecraft:dirt", None),
                Server
            )
            .is_empty()
        );
    }
    // The destroy stays active on the broken block, so the next block continues it.
    assert_eq!(
        kinds(&held(
            &mut machine,
            &target([1, 1, 3], "minecraft:dirt", None),
            Server
        )),
        [(ContinueDestroy, [1, 1, 3], 1)]
    );
}

#[test]
fn server_target_change_is_one_continue_and_release_aborts_with_progress_percent() {
    let first = target([0, 0, 0], "minecraft:stone", None);
    let second = target([0, 0, 1], "minecraft:stone", None);
    let mut machine = DestroyMachine::default();
    held(&mut machine, &first, Server);
    for _ in 0..30 {
        held(&mut machine, &first, Server);
    }
    // Face changes on the same block send nothing.
    let turned = DestroyTarget {
        face: 4,
        ..first.clone()
    };
    assert!(held(&mut machine, &turned, Server).is_empty());
    assert_eq!(
        kinds(&held(&mut machine, &second, Server)),
        [(ContinueDestroy, [0, 0, 1], 1)]
    );
    let rate = second.rate(true).unwrap();
    let mut progress = 0.0_f32;
    for _ in 0..75 {
        held(&mut machine, &second, Server);
        progress += rate;
    }
    let percent = (progress * 100.0) as u8;
    assert!((40..60).contains(&percent));
    assert_eq!(
        kinds(&machine.step(DestroyInput::Released, true, Server)),
        [(AbortDestroy, [0, 0, 1], percent)]
    );
    assert!(
        machine
            .step(DestroyInput::Released, true, Server)
            .is_empty()
    );
    assert_eq!(
        kinds(&held(&mut machine, &first, Server)),
        [(StartDestroy, [0, 0, 0], 1)]
    );
    assert_eq!(
        kinds(&machine.step(DestroyInput::Held(None), true, Server)),
        [(AbortDestroy, [0, 0, 0], 0)]
    );
}

#[test]
fn client_authority_cracks_each_tick_and_completes_with_stop_and_destroy_transaction() {
    let leaves = target(
        [4, 5, 6],
        "minecraft:oak_leaves",
        Some("minecraft:golden_hoe"),
    );
    assert!(leaves.rate(true).unwrap() >= 1.0);
    let mut machine = DestroyMachine::default();
    assert_eq!(
        kinds(&held(&mut machine, &leaves, Client)),
        [(StartDestroy, [4, 5, 6], 1), (CrackBlock, [4, 5, 6], 1)]
    );
    let done = held(&mut machine, &leaves, Client);
    assert_eq!(kinds(&done), [(StopDestroy, [0, 0, 0], 0)]);
    assert_eq!(
        done.destroy.as_ref().map(|target| target.position),
        Some([4, 5, 6])
    );
    let interactions = done.into_interactions([0.5, 64.0, 0.5]);
    assert!(matches!(
        interactions.block_interaction,
        Some(protocol::BlockItemInteraction::Destroy(ref request))
            if request.block_position == [4, 5, 6] && request.selected_slot == 2
    ));
    // A rate at or above one breaks without a following delay.
    let other = target([4, 5, 7], "minecraft:stone", None);
    assert_eq!(
        kinds(&held(&mut machine, &other, Client)),
        [
            (AbortDestroy, [4, 5, 6], 0),
            (StartDestroy, [4, 5, 7], 1),
            (CrackBlock, [4, 5, 7], 1)
        ]
    );
}

#[test]
fn zero_hardness_breaks_on_the_start_tick_and_then_delays() {
    let torch = target([2, 2, 2], "minecraft:torch", None);
    let mut machine = DestroyMachine::default();
    assert_eq!(
        kinds(&held(&mut machine, &torch, Server)),
        [(StartDestroy, [2, 2, 2], 1), (PredictDestroy, [2, 2, 2], 1)]
    );
    let next = target([2, 1, 2], "minecraft:torch", None);
    for _ in 0..DESTROY_DELAY_TICKS {
        assert!(held(&mut machine, &next, Server).is_empty());
    }
    assert!(!held(&mut machine, &next, Server).is_empty());
}

#[test]
fn a_predicted_break_waits_for_its_block_update() {
    let leaves = target(
        [0, 3, 0],
        "minecraft:oak_leaves",
        Some("minecraft:golden_hoe"),
    );
    let mut machine = DestroyMachine::default();
    held(&mut machine, &leaves, Server);
    assert_eq!(
        kinds(&held(&mut machine, &leaves, Server)),
        [
            (ContinueDestroy, [0, 3, 0], 1),
            (PredictDestroy, [0, 3, 0], 1)
        ]
    );
    // The unchanged block is locally gone: no restart and no abort while held.
    for _ in 0..PREDICTED_BREAK_HOLD_TICKS - 1 {
        assert!(held(&mut machine, &leaves, Server).is_empty());
    }
    // Without an update the hold expires and destroying resumes on it.
    assert_eq!(
        kinds(&held(&mut machine, &leaves, Server)),
        [
            (ContinueDestroy, [0, 3, 0], 1),
            (PredictDestroy, [0, 3, 0], 1)
        ]
    );
}

#[test]
fn interruption_aborts_on_the_next_step_only() {
    let stone = target([7, 7, 7], "minecraft:stone", None);
    let mut machine = DestroyMachine::default();
    held(&mut machine, &stone, Server);
    machine.interrupt();
    assert_eq!(
        kinds(&held(&mut machine, &stone, Server)),
        [(AbortDestroy, [7, 7, 7], 0), (StartDestroy, [7, 7, 7], 1)]
    );
    // Unknown blocks keep a destroy open without ever predicting completion.
    let unknown = target([8, 8, 8], "minecraft:not_a_block", None);
    held(&mut machine, &unknown, Server);
    for _ in 0..1_000 {
        assert!(held(&mut machine, &unknown, Server).is_empty());
    }
}

fn completed(tick: u64) -> PhysicsMovementSample {
    PhysicsMovementSample {
        tick,
        position: [0.5, 2.620_01, 0.5],
        velocity: [0.0; 3],
        move_vector: [0.0; 2],
        raw_move_vector: [0.0; 2],
        analogue_move_vector: [0.0; 2],
        pitch: 0.0,
        yaw: 0.0,
        head_yaw: 0.0,
        camera_orientation: [0.0, 0.0, -1.0],
        jumping: false,
        sneaking: false,
        sprinting: false,
        input_mode: PlayerInputMode::Mouse,
        grounded_before_tick: true,
        grounded_after_tick: true,
        horizontal_collision: false,
        vertical_collision: false,
        jump_repeated: false,
        processed: ProcessedMovementState::default(),
        world_identity: sim::CollisionQuery::synthetic(()).identity,
    }
}

fn evidence() -> PhysicsTickEvidenceContext {
    PhysicsTickEvidenceContext {
        fifo_sequence: 19,
        pose_generation: 23,
        dimension: 0,
        perspective: semantic_input::PerspectiveMode::FirstPerson,
        camera_blocked: false,
        camera_fallback: false,
        local_avatar_visible: false,
        look_delta: [0.0; 2],
        outbound_authorized: true,
        outbox_depth: 1,
        outbox_drops: 0,
        free_camera_packet_count: 0,
    }
}

#[test]
fn each_unsent_tick_is_stepped_once_and_survives_creative_revocation() {
    let mut ticker = MovementTicker::default();
    ticker.reset(7, 100, [0.5, 2.620_01, 0.5]);
    ticker.set_source(MovementSource::Physics);
    ticker.testing_lift_spawn_settle_gate();
    for tick in 101..=102 {
        ticker.enqueue_completed_physics(completed(tick)).unwrap();
    }
    let stone = target([0, 1, -3], "minecraft:stone", None);
    let mut runtime = SurvivalMiningRuntime::default();
    let mut swings = Vec::new();
    runtime.step_ticks(
        &mut ticker,
        DestroyInput::Held(Some(&stone)),
        Server,
        |tick| {
            swings.push(tick);
        },
    );
    // Re-running the frame must not step the same ticks again.
    runtime.step_ticks(&mut ticker, DestroyInput::Released, Server, |_| {});
    ticker.retain_creative_mining(None);
    ticker.enqueue_completed_physics(completed(103)).unwrap();
    runtime.step_ticks(&mut ticker, DestroyInput::Released, Server, |tick| {
        swings.push(tick);
    });
    assert_eq!(
        swings,
        [101, 102],
        "each held tick on a block attempts a swing"
    );

    let mut packets = Vec::new();
    flush_player_auth_inputs(&mut ticker, 8, Some(evidence()), |_, packet| {
        packets.push(packet);
        Ok::<_, ()>(())
    })
    .unwrap();
    let carries_actions = packets
        .iter()
        .map(|packet| {
            protocol::player_auth_input_trace_sample(packet)
                .unwrap()
                .flag_names
                .contains(&"PerformBlockActions")
        })
        .collect::<Vec<_>>();
    // Start on the first tick, silence while held, abort once released.
    assert_eq!(carries_actions, [true, false, true]);
}
