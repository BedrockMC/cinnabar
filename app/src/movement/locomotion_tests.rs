//! Locomotion-mode wire edges: each start/stop flag rides exactly the tick the simulator changed mode.

use std::time::Duration;

use protocol::PlayerInputFlags;
use sim::{
    Aabb, CollisionQuery, CollisionWorld, MovementInput, MovementMode, Vec3, WorldQueryError,
};

use super::integration_tests::VersionedFloor;
use super::settle_tests::settled_sample;
use super::{
    HeldInput, LocalPhysicsController, ModeIntent, PhysicsMovementSample, PhysicsSampleContext,
    RideKind, input_flags,
};

const TICK: Duration = Duration::from_millis(50);

/// Floor top at y=1 plus a ceiling whose underside sits at the given height.
struct LowCeiling(f64);

impl CollisionWorld for LowCeiling {
    fn collision_boxes(&self, query: Aabb) -> Result<CollisionQuery<Vec<Aabb>>, WorldQueryError> {
        let mut base = VersionedFloor(1).collision_boxes(query)?;
        let ceiling = Aabb::new(
            Vec3::new(-64.0, self.0, -64.0),
            Vec3::new(64.0, self.0 + 1.0, 64.0),
        );
        if ceiling.intersects(query) {
            base.value.push(ceiling);
        }
        Ok(base)
    }

    fn block_physics(&self, block: [i32; 3]) -> Result<sim::BlockPhysicsSample, WorldQueryError> {
        VersionedFloor(1).block_physics(block)
    }
}

fn grounded_controller() -> LocalPhysicsController {
    let mut physics = LocalPhysicsController::default();
    physics.reanchor_network_position([0.0, 2.620_01, 0.0], 100, true);
    physics
}

/// Consumes the fresh-anchor depenetration probe on open ground so later ticks start clean.
fn settled_controller() -> LocalPhysicsController {
    let mut physics = grounded_controller();
    step(
        &mut physics,
        MovementInput::default(),
        ModeIntent::default(),
        &VersionedFloor(1),
    );
    physics
}

fn step(
    physics: &mut LocalPhysicsController,
    input: MovementInput,
    intent: ModeIntent,
    world: &impl CollisionWorld,
) -> PhysicsMovementSample {
    let frame = physics.advance_with_context(
        TICK,
        input,
        PhysicsSampleContext {
            mode_intent: intent,
            ..PhysicsSampleContext::default()
        },
        world,
    );
    assert!(frame.blocked.is_none(), "blocked: {:?}", frame.blocked);
    frame.samples.into_iter().next().expect("one tick")
}

fn has(flags: PlayerInputFlags, flag: PlayerInputFlags) -> bool {
    flags.bits() & flag.bits() != 0
}

#[test]
fn flight_start_ascend_and_stop_edges_follow_the_simulated_mode() {
    let mut physics = grounded_controller();
    let can_fly = ModeIntent {
        can_fly: true,
        ..ModeIntent::default()
    };
    let hold_jump = MovementInput {
        jumping: true,
        ..MovementInput::default()
    };
    let world = VersionedFloor(1);

    let start = step(
        &mut physics,
        hold_jump,
        ModeIntent {
            fly_toggle: true,
            ..can_fly
        },
        &world,
    );
    assert_eq!(start.processed.mode, MovementMode::Flying);
    let start_flags = input_flags(&start, HeldInput::default());
    assert!(has(start_flags, PlayerInputFlags::START_FLYING));
    assert!(has(start_flags, PlayerInputFlags::ASCEND));

    let cruise = step(&mut physics, hold_jump, can_fly, &world);
    assert_eq!(cruise.processed.mode, MovementMode::Flying);
    let cruise_flags = input_flags(&cruise, HeldInput::from(&start));
    assert!(!has(cruise_flags, PlayerInputFlags::START_FLYING));
    assert!(cruise.position[1] > start.position[1]);

    let stop = step(
        &mut physics,
        MovementInput::default(),
        ModeIntent {
            fly_toggle: true,
            ..can_fly
        },
        &world,
    );
    assert_eq!(stop.processed.mode, MovementMode::Walking);
    let stop_flags = input_flags(&stop, HeldInput::from(&cruise));
    assert!(has(stop_flags, PlayerInputFlags::STOP_FLYING));
    assert!(!has(stop_flags, PlayerInputFlags::ASCEND));
}

#[test]
fn flight_without_permission_never_starts() {
    let mut physics = grounded_controller();
    let sample = step(
        &mut physics,
        MovementInput::default(),
        ModeIntent {
            fly_toggle: true,
            ..ModeIntent::default()
        },
        &VersionedFloor(1),
    );
    assert_eq!(sample.processed.mode, MovementMode::Walking);
}

#[test]
fn crawl_edges_fire_once_on_entry_and_once_on_exit() {
    let mut crawling = settled_sample(101, [0.0, 2.620_01, 0.0]);
    crawling.processed.mode = MovementMode::Crawling;
    let entry = input_flags(&crawling, HeldInput::default());
    assert!(has(entry, PlayerInputFlags::START_CRAWLING));

    let steady = input_flags(&crawling, HeldInput::from(&crawling));
    assert!(!has(steady, PlayerInputFlags::START_CRAWLING));
    assert!(!has(steady, PlayerInputFlags::STOP_CRAWLING));

    let standing = settled_sample(102, [0.0, 2.620_01, 0.0]);
    let exit = input_flags(&standing, HeldInput::from(&crawling));
    assert!(has(exit, PlayerInputFlags::STOP_CRAWLING));
}

#[test]
fn swim_and_glide_edges_pair_start_with_stop() {
    for (mode, start, stop) in [
        (
            MovementMode::Swimming,
            PlayerInputFlags::START_SWIMMING,
            PlayerInputFlags::STOP_SWIMMING,
        ),
        (
            MovementMode::Gliding,
            PlayerInputFlags::START_GLIDING,
            PlayerInputFlags::STOP_GLIDING,
        ),
    ] {
        let mut active = settled_sample(101, [0.0, 2.620_01, 0.0]);
        active.processed.mode = mode;
        assert!(has(input_flags(&active, HeldInput::default()), start));
        let idle = settled_sample(102, [0.0, 2.620_01, 0.0]);
        assert!(has(input_flags(&idle, HeldInput::from(&active)), stop));
    }
}

#[test]
fn a_ceiling_that_only_fits_a_sneak_forces_the_pose_and_persist_flag() {
    let mut physics = settled_controller();
    let world = LowCeiling(2.6);
    let first = step(
        &mut physics,
        MovementInput::default(),
        ModeIntent::default(),
        &world,
    );
    assert!(first.processed.forced_sneak && first.sneaking);
    let flags = input_flags(&first, HeldInput::default());
    assert!(has(flags, PlayerInputFlags::PERSIST_SNEAK));
    assert!(has(flags, PlayerInputFlags::SNEAKING));
    assert_eq!(first.processed.mode, MovementMode::Walking);
}

fn riding_sample(kind: RideKind, move_vector: [f32; 2]) -> PhysicsMovementSample {
    let mut sample = settled_sample(101, [0.0, 2.620_01, 0.0]);
    sample.processed.mode = MovementMode::Riding;
    sample.processed.ride = Some(kind);
    sample.move_vector = move_vector;
    sample
}

#[test]
fn boat_paddles_follow_steering_and_other_rides_never_paddle() {
    let paddles = |kind, vector| {
        let flags = input_flags(&riding_sample(kind, vector), HeldInput::default());
        (
            has(flags, PlayerInputFlags::PADDLING_LEFT),
            has(flags, PlayerInputFlags::PADDLING_RIGHT),
        )
    };
    assert_eq!(paddles(RideKind::Boat, [0.0, 1.0]), (true, true));
    assert_eq!(paddles(RideKind::Boat, [-1.0, 0.0]), (true, false));
    assert_eq!(paddles(RideKind::Boat, [1.0, 0.0]), (false, true));
    assert_eq!(paddles(RideKind::Boat, [0.0, 0.0]), (false, false));
    assert_eq!(paddles(RideKind::Horse, [0.0, 1.0]), (false, false));
}

#[test]
fn a_mounted_controller_streams_a_frozen_pose_with_steering_intact() {
    let mut physics = settled_controller();
    let rider = ModeIntent {
        ride: Some(RideKind::Horse),
        ..ModeIntent::default()
    };
    let before = physics.network_position().unwrap();
    let sample = step(
        &mut physics,
        MovementInput {
            forward: 1.0,
            sprinting: true,
            ..MovementInput::default()
        },
        rider,
        &VersionedFloor(1),
    );
    assert_eq!(sample.processed.mode, MovementMode::Riding);
    assert_eq!(sample.position, before);
    assert_eq!(sample.movement, [0.0; 3]);
    assert!(!sample.processed.sprinting);
    assert_eq!(sample.move_vector[1], 1.0);
}

#[test]
fn a_rider_follows_the_mount_seat_and_reports_the_seat_delta() {
    let mut physics = settled_controller();
    let before = physics.network_position().unwrap();
    let rider = ModeIntent {
        ride: Some(RideKind::Boat),
        ride_seat: Some([5.0, 3.0, 5.0]),
        ..ModeIntent::default()
    };
    let sample = step(
        &mut physics,
        MovementInput::default(),
        rider,
        &VersionedFloor(1),
    );
    assert_eq!(sample.position[0], 5.0);
    assert_eq!(sample.position[2], 5.0);
    assert!((sample.position[1] - (3.0 + protocol::PLAYER_NETWORK_OFFSET)).abs() < 1.0e-4);
    assert!((sample.movement[0] - (5.0 - before[0])).abs() < 1.0e-5);
    assert!(!has(
        input_flags(&sample, HeldInput::default()),
        PlayerInputFlags::JUMPING
    ));
}
