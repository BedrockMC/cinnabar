#[test]
fn nonbinary_primary_bits_and_captured_directions_survive_replay_replacement() {
    let mut physics = LocalPhysicsController::default();
    physics.reanchor_network_position([0.0, 2.620_01, 0.0], 100, true);
    let mut input = physics_movement_input([0.7, 0.9], 0.0, true, false, true, false, None);
    input.item_use_movement_modifier = Some(f64::from(0.7_f32));
    let frame = physics.advance_with_context(
        Duration::from_millis(150),
        input,
        PhysicsSampleContext {
            raw_move_vector: [-1.0, -1.0],
            analogue_move_vector: [-1.0, -1.0],
            ..Default::default()
        },
        &VersionedFloor(1),
    );
    assert_eq!(frame.samples.len(), 3);
    let factor = 0.7_f32 * 0.3_f32;
    let expected = [(0.7_f32 * factor).to_bits(), (0.9_f32 * factor).to_bits()];
    let wire_expected = [(-0.7_f32 * factor).to_bits(), (0.9_f32 * factor).to_bits()];
    let mut ticker = MovementTicker::default();
    ticker.reset(7, 100, [0.0, 2.620_01, 0.0]);
    ticker.set_source(MovementSource::Physics);
    for sample in frame.samples {
        ticker.enqueue_completed_physics(sample).unwrap();
    }
    ticker.pop_pending().unwrap();
    let before = ticker.pending_samples();
    let confirmation = ticker.sent_confirmation(101);
    let plan = physics
        .clone()
        .apply_correction(
            super::PhysicsAnchor {
                network_position: [0.25, 2.620_01, 0.0],
                tick: 101,
                on_ground: true,
                velocity: None,
            },
            PhysicsCorrectionMode::ReplayIfRetained,
            confirmation.as_ref(),
            &VersionedFloor(1),
        )
        .unwrap();
    assert_eq!(
        plan.outcome,
        PhysicsCorrectionOutcome::Replayed {
            corrected_tick: 101,
            replayed_ticks: 2,
        }
    );
    assert_eq!(plan.replayed_samples.len(), before.len());
    let mask = PlayerInputFlags::UP | PlayerInputFlags::RIGHT | PlayerInputFlags::UP_LEFT;
    for (live, retained) in before.iter().zip(&plan.replayed_samples) {
        assert_eq!(retained.tick, live.snapshot.tick);
        assert_eq!(retained.move_vector.map(f32::to_bits), expected);
        assert_eq!(
            retained.processed.direction_flags.unwrap().bits() & mask.bits(),
            live.snapshot.flags.bits() & mask.bits()
        );
        assert_eq!(retained.raw_move_vector, [-1.0, -1.0]);
        assert_eq!(retained.analogue_move_vector, [-1.0, -1.0]);
    }
    // Captured input stays immutable; only replay-owned output is corrupted.
    for (pending, retained) in ticker.outbox.iter_mut().zip(&plan.replayed_samples) {
        pending.snapshot.position = [99.0; 3];
        pending.snapshot.delta = [99.0; 3];
        pending.snapshot.flags = pending
            .snapshot
            .flags
            .with_mask(
                PlayerInputFlags::HORIZONTAL_COLLISION,
                !retained.horizontal_collision,
            )
            .with_mask(
                PlayerInputFlags::VERTICAL_COLLISION,
                !retained.vertical_collision,
            )
            .with_mask(
                PlayerInputFlags::JUMPING,
                !retained.processed.jump_arc_active,
            );
    }
    reconcile_candidate_physics_correction(
        &mut ticker,
        &mut physics,
        [0.25, 2.620_01, 0.0],
        101,
        true,
        PhysicsCorrectionMode::ReplayIfRetained,
        &VersionedFloor(1),
    )
    .unwrap();
    let after = ticker.pending_samples();
    assert_eq!(before.len(), 2);
    assert_eq!(after.len(), before.len());
    for ((live, replayed), retained) in before.into_iter().zip(after).zip(plan.replayed_samples) {
        assert_eq!(replayed.snapshot.position, retained.position);
        assert_eq!(replayed.snapshot.delta, retained.velocity);
        assert_ne!(replayed.snapshot.position, [99.0; 3]);
        assert_ne!(replayed.snapshot.delta, [99.0; 3]);
        assert_ne!(replayed.snapshot.position, live.snapshot.position);
        assert_eq!(replayed.evidence.network_position, retained.position);
        for (flag, expected_value) in [
            (
                PlayerInputFlags::HORIZONTAL_COLLISION,
                retained.horizontal_collision,
            ),
            (
                PlayerInputFlags::VERTICAL_COLLISION,
                retained.vertical_collision,
            ),
            (
                PlayerInputFlags::JUMPING,
                retained.processed.jump_arc_active,
            ),
        ] {
            assert_eq!(
                replayed.snapshot.flags.bits() & flag.bits() != 0,
                expected_value
            );
        }
        assert_eq!(live.snapshot.move_vector.map(f32::to_bits), wire_expected);
        assert_eq!(replayed.snapshot.move_vector.map(f32::to_bits), wire_expected);
        assert_eq!(
            replayed.snapshot.flags.bits() & mask.bits(),
            live.snapshot.flags.bits() & mask.bits()
        );
        assert_eq!(
            replayed.snapshot.flags.bits() & PlayerInputFlags::UP_LEFT.bits(),
            0
        );
        assert_eq!(replayed.snapshot.raw_move_vector, [1.0, -1.0]);
        assert_eq!(replayed.snapshot.analogue_move_vector, [1.0, -1.0]);
    }
}
