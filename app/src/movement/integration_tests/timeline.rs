// Tick-stamped authoritative updates entering the rewind timeline.

fn walked_physics(ticks: u64) -> (LocalPhysicsController, MovementTicker) {
    let mut physics = LocalPhysicsController::default();
    physics.reanchor_network_position([0.0, 2.620_01, 0.0], 100, true);
    let mut ticker = MovementTicker::default();
    ticker.reset(7, 100, [0.0, 2.620_01, 0.0]);
    ticker.set_source(MovementSource::Physics);
    for _ in 0..ticks {
        let sample = run_one_tick(&mut physics, &VersionedFloor(1));
        ticker.enqueue_completed_physics(sample).unwrap();
    }
    (physics, ticker)
}

/// Delayed knockback must rewind to its tick instead of waiting for a coincidental replay.
#[test]
fn delayed_server_motion_rewinds_to_its_tick_and_matches_on_time_delivery() {
    let motion = [0.45, 0.42, -0.35];
    let (mut on_time, _) = walked_physics(2);
    assert_eq!(on_time.queue_server_motion(motion, 102), None);
    run_one_tick(&mut on_time, &VersionedFloor(1));
    run_one_tick(&mut on_time, &VersionedFloor(1));

    let (mut delayed, mut ticker) = walked_physics(4);
    assert_eq!(delayed.queue_server_motion(motion, 102), Some(102));
    let outcome =
        reconcile_timeline_rewind(&mut ticker, &mut delayed, 102, &VersionedFloor(1)).unwrap();
    assert_eq!(
        outcome,
        PhysicsCorrectionOutcome::Replayed {
            corrected_tick: 102,
            replayed_ticks: 2,
        }
    );
    assert_eq!(delayed.state(), on_time.state());
    let pending: Vec<_> = ticker
        .pending_samples()
        .iter()
        .map(|pending| pending.snapshot.position)
        .collect();
    assert_eq!(
        pending.last(),
        delayed.network_position().as_ref(),
        "unsent samples carry the replayed knockback"
    );
}

#[test]
fn stale_server_motion_clamps_to_the_oldest_retained_frame() {
    let (mut physics, _) = walked_physics(3);
    physics.reanchor_network_position([0.0, 2.620_01, 0.0], 200, true);
    for _ in 0..3 {
        run_one_tick(&mut physics, &VersionedFloor(1));
    }
    assert_eq!(
        physics.queue_server_motion([0.1, 0.0, 0.0], 150),
        Some(201),
        "older than history: rewind from the oldest retained frame"
    );
}

#[test]
fn current_and_future_server_motion_apply_to_live_state() {
    let (mut physics, _) = walked_physics(2);
    assert_eq!(physics.queue_server_motion([0.3, 0.0, 0.0], 102), None);
    assert_eq!(physics.state().unwrap().velocity.x, f64::from(0.3_f32));
    assert_eq!(physics.queue_server_motion([-0.2, 0.0, 0.0], 150), None);
    assert_eq!(physics.state().unwrap().velocity.x, f64::from(-0.2_f32));
}
