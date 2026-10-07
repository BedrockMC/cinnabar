//! Prediction packet placement, replay distance and deferred motion ordering.

use super::*;

#[test]
fn a_large_prediction_correction_replays_retained_ticks_without_teleporting() {
    let world = VersionedFloor(1);
    let mut physics = LocalPhysicsController::default();
    physics.reanchor_network_position([0.0, 2.620_01, 0.0], 100, true);
    let frame = physics.advance_with_context(
        Duration::from_millis(150),
        forward_physics_input(),
        PhysicsSampleContext::default(),
        &world,
    );
    let anchor = &frame.samples[0];
    let mut position = anchor.position;
    position[0] += 32.0;
    let mut ticker = ticker_with_samples(frame.samples.iter().cloned());
    let outcome = crate::movement::reconcile_prediction_correction(
        &mut ticker,
        &mut physics,
        position,
        anchor.tick,
        anchor.grounded_after_tick,
        anchor.velocity,
        &world,
    )
    .unwrap();
    assert_eq!(
        outcome,
        Some(PhysicsCorrectionOutcome::Replayed {
            corrected_tick: anchor.tick,
            replayed_ticks: 2,
        })
    );
    assert_eq!(physics.state().unwrap().tick, 103);
    assert!(physics.retains_tick(anchor.tick));
    assert_eq!(ticker.pending_count(), 2);
}

#[test]
fn a_future_prediction_correction_waits_on_the_current_frame_for_a_later_rewind() {
    let world = VersionedFloor(1);
    let mut physics = LocalPhysicsController::default();
    physics.reanchor_network_position([0.0, 2.620_01, 0.0], 100, true);
    let frame = physics.advance_with_context(
        Duration::from_millis(150),
        forward_physics_input(),
        PhysicsSampleContext::default(),
        &world,
    );
    let mut ticker = ticker_with_samples(frame.samples.iter().cloned());
    let before = physics.state().cloned();
    let outcome = crate::movement::reconcile_prediction_correction(
        &mut ticker,
        &mut physics,
        [30.0, 2.62, 0.0],
        500,
        true,
        [0.0; 3],
        &world,
    )
    .unwrap();
    assert_eq!(outcome, None);
    assert_eq!(physics.state().cloned(), before);
    assert_eq!(ticker.pending_count(), 3);

    let anchor = &frame.samples[0];
    let position = anchor.position;
    let outcome = crate::movement::reconcile_prediction_correction(
        &mut ticker,
        &mut physics,
        position,
        anchor.tick,
        true,
        anchor.velocity,
        &world,
    )
    .unwrap();
    assert!(matches!(
        outcome,
        Some(PhysicsCorrectionOutcome::Replayed {
            replayed_ticks: 2,
            ..
        })
    ));
    assert_eq!(physics.state().unwrap().tick, 103);
    assert_eq!(physics.state().unwrap().position.x, 30.0);
    assert_eq!(physics.state().unwrap().velocity.x, 0.0);
}

#[test]
fn a_later_correction_to_the_same_frame_supersedes_the_deferred_correction() {
    let world = VersionedFloor(1);
    let mut physics = LocalPhysicsController::default();
    physics.reanchor_network_position([0.0, 2.620_01, 0.0], 100, true);
    let frame = physics.advance_with_context(
        Duration::from_millis(150),
        forward_physics_input(),
        PhysicsSampleContext::default(),
        &world,
    );
    let mut ticker = ticker_with_samples(frame.samples.iter().cloned());
    crate::movement::reconcile_prediction_correction(
        &mut ticker,
        &mut physics,
        [30.0, 2.62, 0.0],
        500,
        true,
        [0.0; 3],
        &world,
    )
    .unwrap();
    let anchor = &frame.samples[1];
    let mut position = anchor.position;
    position[0] += 0.01;
    crate::movement::reconcile_prediction_correction(
        &mut ticker,
        &mut physics,
        position,
        anchor.tick,
        true,
        anchor.velocity,
        &world,
    )
    .unwrap();
    assert!((physics.state().unwrap().position.x - f64::from(position[0])).abs() < 1.0e-7);
}

#[test]
fn later_server_motion_overrides_deferred_velocity_on_the_same_frame() {
    let world = VersionedFloor(1);
    let mut physics = LocalPhysicsController::default();
    physics.reanchor_network_position([0.0, 2.620_01, 0.0], 100, true);
    let frame = physics.advance_with_context(
        Duration::from_millis(150),
        forward_physics_input(),
        PhysicsSampleContext::default(),
        &world,
    );
    let mut ticker = ticker_with_samples(frame.samples.iter().cloned());
    crate::movement::reconcile_prediction_correction(
        &mut ticker,
        &mut physics,
        [30.0, 3.0, 0.0],
        500,
        false,
        [0.0; 3],
        &world,
    )
    .unwrap();
    let tick = physics.queue_server_motion([0.5, 0.1, 0.0], 102).unwrap();
    reconcile_timeline_rewind(&mut ticker, &mut physics, tick, &world).unwrap();
    assert!(physics.state().unwrap().position.x > 30.4);
}

/// A deferred relocation cannot carry a wall collision into an unobstructed ladder tick.
#[test]
fn a_deferred_prediction_correction_clears_collision_flags_before_ladder_replay() {
    let world = ClimbableWall(VersionedWall(1));
    let (mut physics, frame) = collided_prediction(&world);
    let anchor = &frame.samples[0];
    assert!(frame.samples[frame.samples.len() - 2].horizontal_collision);
    let final_tick = physics.state().unwrap().tick;
    let mut ticker = ticker_with_samples(frame.samples.iter().cloned());
    let destination = [0.0, 3.0 + protocol::PLAYER_NETWORK_OFFSET, 8.0];
    assert_eq!(
        crate::movement::reconcile_prediction_correction(
            &mut ticker,
            &mut physics,
            destination,
            500,
            false,
            [0.0; 3],
            &world,
        )
        .unwrap(),
        None
    );
    assert!(matches!(
        crate::movement::reconcile_prediction_correction(
            &mut ticker,
            &mut physics,
            anchor.position,
            anchor.tick,
            anchor.grounded_after_tick,
            anchor.velocity,
            &world,
        )
        .unwrap(),
        Some(PhysicsCorrectionOutcome::Replayed { .. })
    ));
    let replayed = physics.sample_at(final_tick).unwrap();
    assert!(!replayed.horizontal_collision);
    assert!(
        replayed.movement[1] <= 0.0,
        "the old wall collision invented a ladder ascent: {:?}",
        replayed.movement
    );
    assert!(replayed.velocity[1] < 0.0);
    assert!(replayed.position[1] <= destination[1]);
}
