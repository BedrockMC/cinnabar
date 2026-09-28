use sim::{
    Aabb, BlockPhysicsFacts, BlockPhysicsFlags, BlockPhysicsSample, CollisionQuery, CollisionWorld,
    MovementInput, MovementMode, PlayerState, ProvenancedCollider, Simulator, SurfaceResponse,
    Vec3, WorldQueryError, pose_fits,
};

/// Fixed solids with optional per-world block flags on every sampled cell.
struct Solids {
    boxes: Vec<(Aabb, [i32; 3])>,
    flags: BlockPhysicsFlags,
}

impl CollisionWorld for Solids {
    fn collision_boxes(&self, query: Aabb) -> Result<CollisionQuery<Vec<Aabb>>, WorldQueryError> {
        Ok(CollisionQuery::synthetic(
            self.boxes
                .iter()
                .map(|(shape, _)| *shape)
                .filter(|shape| shape.intersects(query.grown(0.01)))
                .collect(),
        ))
    }

    fn collision_boxes_with_provenance(
        &self,
        query: Aabb,
    ) -> Result<CollisionQuery<Vec<ProvenancedCollider>>, WorldQueryError> {
        Ok(CollisionQuery::synthetic(
            self.boxes
                .iter()
                .filter(|(shape, _)| shape.intersects(query.grown(0.01)))
                .map(|(shape, block)| ProvenancedCollider {
                    aabb: *shape,
                    block: Some(*block),
                    runtime_id: Some(1),
                })
                .collect(),
        ))
    }

    fn block_physics(&self, _block: [i32; 3]) -> Result<BlockPhysicsSample, WorldQueryError> {
        Ok(BlockPhysicsSample {
            layers: Box::new([BlockPhysicsFacts {
                friction: 0.6,
                horizontal_speed_factor: 1.0,
                vertical_speed_factor: 1.0,
                fluid_height_blocks: 0.0,
                flags: self.flags,
                surface_response: SurfaceResponse::None,
            }]),
            identity: CollisionQuery::synthetic(()).identity,
        })
    }
}

fn empty(flags: BlockPhysicsFlags) -> Solids {
    Solids {
        boxes: Vec::new(),
        flags,
    }
}

fn unit_block() -> Aabb {
    Aabb::new(Vec3::new(-8.0, 0.0, -8.0), Vec3::new(8.0, 1.0, 8.0))
}

fn tick(state: &mut PlayerState, input: MovementInput, world: &Solids) -> sim::TickResult {
    Simulator::default().tick(state, input, world).unwrap()
}

#[test]
fn flying_hovers_without_gravity_and_jump_sneak_steer_vertically() {
    let world = empty(BlockPhysicsFlags::default());
    let mut state = PlayerState::new(Vec3::new(0.5, 10.0, 0.5));
    let hover = tick(
        &mut state,
        MovementInput {
            mode: MovementMode::Flying,
            ..MovementInput::default()
        },
        &world,
    );
    assert_eq!(hover.movement.y, 0.0);

    let up = tick(
        &mut state,
        MovementInput {
            mode: MovementMode::Flying,
            jumping: true,
            ..MovementInput::default()
        },
        &world,
    );
    assert!(up.movement.y > 0.0);

    let mut state = PlayerState::new(Vec3::new(0.5, 10.0, 0.5));
    let down = tick(
        &mut state,
        MovementInput {
            mode: MovementMode::Flying,
            sneaking: true,
            ..MovementInput::default()
        },
        &world,
    );
    assert!(down.movement.y < 0.0);
}

#[test]
fn faster_ability_speed_flies_farther() {
    let world = empty(BlockPhysicsFlags::default());
    let travel = |fly_speed| {
        let mut state = PlayerState::new(Vec3::new(0.5, 10.0, 0.5));
        tick(
            &mut state,
            MovementInput {
                mode: MovementMode::Flying,
                forward: 1.0,
                fly_speed,
                ..MovementInput::default()
            },
            &world,
        )
        .movement
        .z
        .abs()
    };
    assert!(travel(Some(0.1)) > travel(None));
}

#[test]
fn gliding_descends_faster_while_diving() {
    let world = empty(BlockPhysicsFlags::default());
    let mut state = PlayerState::new(Vec3::new(0.5, 50.0, 0.5));
    state.velocity = Vec3::new(0.0, 0.0, 0.5);
    let mut previous_y = 0.0;
    for _ in 0..5 {
        let result = tick(
            &mut state,
            MovementInput {
                mode: MovementMode::Gliding,
                pitch_degrees: 50.0,
                ..MovementInput::default()
            },
            &world,
        );
        assert!(result.movement.y < previous_y);
        previous_y = result.movement.y;
    }
}

#[test]
fn low_poses_fit_a_one_block_gap_that_standing_does_not() {
    let world = Solids {
        boxes: vec![
            (
                Aabb::new(Vec3::new(-8.0, -1.0, -8.0), Vec3::new(8.0, 0.0, 8.0)),
                [0, -1, 0],
            ),
            (
                Aabb::new(Vec3::new(-8.0, 1.0, -8.0), Vec3::new(8.0, 2.0, 8.0)),
                [0, 1, 0],
            ),
        ],
        flags: BlockPhysicsFlags::default(),
    };
    let feet = Vec3::new(0.5, 0.0, 0.5);
    assert!(!pose_fits(&world, feet, MovementMode::Walking, false).unwrap());
    assert!(!pose_fits(&world, feet, MovementMode::Walking, true).unwrap());
    assert!(pose_fits(&world, feet, MovementMode::Crawling, false).unwrap());
    assert!(pose_fits(&world, feet, MovementMode::Swimming, false).unwrap());
}

#[test]
fn scaffolding_supports_from_above_and_is_passable_when_sneaking_or_beside() {
    let world = Solids {
        boxes: vec![(unit_block(), [0, 0, 0])],
        flags: BlockPhysicsFlags::SCAFFOLDING,
    };
    let mut standing = PlayerState::new(Vec3::new(0.5, 1.0, 0.5));
    standing.velocity.y = -0.3;
    let landed = tick(&mut standing, MovementInput::default(), &world);
    assert!(landed.movement.y.abs() < 1.0e-9);

    let mut sneaking = PlayerState::new(Vec3::new(0.5, 1.0, 0.5));
    let sunk = tick(
        &mut sneaking,
        MovementInput {
            sneaking: true,
            ..MovementInput::default()
        },
        &world,
    );
    assert!(sunk.movement.y < 0.0);

    let mut inside = PlayerState::new(Vec3::new(0.5, 0.4, 0.5));
    let walked = tick(
        &mut inside,
        MovementInput {
            forward: 1.0,
            yaw_degrees: 0.0,
            ..MovementInput::default()
        },
        &world,
    );
    assert!(!walked.collisions.z);
}

#[test]
fn creative_hover_damps_harder_and_vertical_speed_scales_ascent() {
    let world = empty(BlockPhysicsFlags::default());
    let second_tick_y = |creative_flight| {
        let mut state = PlayerState::new(Vec3::new(0.5, 10.0, 0.5));
        state.velocity.y = 1.0;
        let input = MovementInput {
            mode: MovementMode::Flying,
            creative_flight,
            ..MovementInput::default()
        };
        tick(&mut state, input, &world);
        tick(&mut state, input, &world).movement.y
    };
    assert!(second_tick_y(true) < second_tick_y(false));

    let ascend = |vertical_fly_speed| {
        let mut state = PlayerState::new(Vec3::new(0.5, 10.0, 0.5));
        tick(
            &mut state,
            MovementInput {
                mode: MovementMode::Flying,
                jumping: true,
                vertical_fly_speed,
                ..MovementInput::default()
            },
            &world,
        )
        .movement
        .y
    };
    assert!(ascend(Some(2.0)) > ascend(None));
}
