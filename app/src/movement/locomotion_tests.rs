//! Locomotion-mode wire edges: each start/stop flag rides exactly the tick the simulator changed mode.

use std::time::Duration;

use protocol::PlayerInputFlags;
use sim::{
    Aabb, CollisionQuery, CollisionWorld, MovementInput, MovementMode, Vec3, WorldQueryError,
};

use super::integration_tests::VersionedFloor;
use super::{
    HeldInput, LocalPhysicsController, ModeIntent, PhysicsMovementSample, PhysicsSampleContext,
    input_flags,
};

const TICK: Duration = Duration::from_millis(50);

/// Floor top at y=1 plus a ceiling leaving a 1.2-high gap: too low to stand or sneak, enough to crawl.
struct LowGap;

impl CollisionWorld for LowGap {
    fn collision_boxes(&self, query: Aabb) -> Result<CollisionQuery<Vec<Aabb>>, WorldQueryError> {
        let mut base = VersionedFloor(1).collision_boxes(query)?;
        let ceiling = Aabb::new(Vec3::new(-64.0, 2.2, -64.0), Vec3::new(64.0, 3.2, 64.0));
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
fn a_gap_too_low_to_stand_in_crawls_and_signals_start_crawling_once() {
    let mut physics = settled_controller();
    let first = step(
        &mut physics,
        MovementInput::default(),
        ModeIntent::default(),
        &LowGap,
    );
    assert_eq!(first.processed.mode, MovementMode::Crawling);
    let first_flags = input_flags(&first, HeldInput::default());
    assert!(has(first_flags, PlayerInputFlags::START_CRAWLING));

    let second = step(
        &mut physics,
        MovementInput::default(),
        ModeIntent::default(),
        &LowGap,
    );
    assert_eq!(second.processed.mode, MovementMode::Crawling);
    let second_flags = input_flags(&second, HeldInput::from(&first));
    assert!(!has(second_flags, PlayerInputFlags::START_CRAWLING));
    assert!(!has(second_flags, PlayerInputFlags::STOP_CRAWLING));
}

#[test]
fn crawling_forces_sprint_off_so_no_sprint_flags_are_asserted() {
    let mut physics = settled_controller();
    let sample = step(
        &mut physics,
        MovementInput {
            forward: 1.0,
            sprinting: true,
            ..MovementInput::default()
        },
        ModeIntent::default(),
        &LowGap,
    );
    assert!(!sample.processed.sprinting);
}
