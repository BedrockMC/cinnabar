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
    input_flags,
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
