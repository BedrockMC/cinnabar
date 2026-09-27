use super::{pose::LocalDelta, query::QueryClock, *};
use crate::ActorPose;

#[test]
fn static_generation_exhaustion_fails_closed_without_consuming_animated_generation() {
    let mut store = ActorAnimationStore::diagnostic();
    let animated = store.next_reset_generation;
    store.next_rest_reset_generation = u64::MAX - 1;
    assert_eq!(store.take_rest_generation(), Some(u64::MAX - 1));
    assert_eq!(store.take_rest_generation(), None);
    assert_eq!(store.take_rest_generation(), None);
    assert_eq!(store.next_reset_generation, animated);
    store.clear();
    assert_eq!(store.take_rest_generation(), None);
}

fn actor_with_metadata(metadata: HashMap<u32, ActorMetadataValue>) -> ActorSnapshot {
    let pose = ActorPose {
        position: [0.0; 3],
        pitch: 0.0,
        yaw: 0.0,
        head_yaw: 0.0,
    };
    ActorSnapshot {
        unique_id: -1,
        runtime_id: 1,
        spawn_revision: 1,
        movement_revision: 0,
        kind: ActorKind::Entity {
            identifier: "minecraft:test".into(),
        },
        position: [0.0; 3],
        velocity: [0.0; 3],
        pitch: 0.0,
        yaw: 0.0,
        head_yaw: 0.0,
        previous_pose: pose,
        received_pose: pose,
        interpolation_ticks_remaining: 0,
        body_yaw: 0.0,
        on_ground: Some(false),
        teleported: false,
        player_mode: None,
        source_tick: None,
        metadata,
        attributes: HashMap::new(),
        int_properties: HashMap::new(),
        float_properties: HashMap::new(),
    }
}

fn read(actor: &ActorSnapshot, input: &ActorTickInput, life_tick: u64, name: &str) -> f32 {
    let clock = QueryClock {
        anim_tick: 0,
        life_tick,
    };
    query::query(actor, input, clock, name, None)
}

#[test]
fn child_before_parent_composes_without_reindexing_channels() {
    let bones = [
        RuntimeBone {
            parent: Some(1),
            pivot: [0.0, 2.0, 0.0],
            rotation: [0.0; 3],
        },
        RuntimeBone {
            parent: None,
            pivot: [1.0, 0.0, 0.0],
            rotation: [0.0; 3],
        },
    ];
    let pose = compose_pose(&bones, &[]).unwrap();
    assert_eq!(pose[0].translation_scale[0..3], [0.0, 2.0, 0.0]);
    assert_eq!(pose[1].translation_scale[0..3], [1.0, 0.0, 0.0]);
}

#[test]
fn rotated_parent_uses_child_model_space_pivot_delta() {
    let bones = [
        RuntimeBone {
            parent: None,
            pivot: [1.0, 0.0, 0.0],
            rotation: [0.0, 0.0, 90.0],
        },
        RuntimeBone {
            parent: Some(0),
            pivot: [3.0, 0.0, 0.0],
            rotation: [0.0; 3],
        },
    ];
    let pose = compose_pose(&bones, &[]).unwrap();
    assert!((pose[1].translation_scale[0] - 1.0).abs() < 1.0e-5);
    assert!((pose[1].translation_scale[1] - 2.0).abs() < 1.0e-5);
}

#[test]
fn nonuniform_scale_is_rejected_instead_of_silently_truncated() {
    let bones = [RuntimeBone {
        parent: None,
        pivot: [0.0; 3],
        rotation: [0.0; 3],
    }];
    let local = [LocalDelta {
        scale: [1.0, 2.0, 3.0],
        ..LocalDelta::default()
    }];
    assert!(compose_pose(&bones, &local).is_none());
}

#[test]
fn sleeping_player_metadata_does_not_spoof_sneaking() {
    let actor = actor_with_metadata(HashMap::from([(26, ActorMetadataValue::Byte(2))]));
    let input = ActorTickInput::default();
    assert_eq!(read(&actor, &input, 0, "query.is_sleeping"), 1.0);
    assert_eq!(read(&actor, &input, 0, "query.is_sneaking"), 0.0);
}

#[test]
fn riding_flag_is_not_read_as_sleeping() {
    let actor = actor_with_metadata(HashMap::from([(0, ActorMetadataValue::Flags(1 << 2))]));
    assert_eq!(
        read(&actor, &ActorTickInput::default(), 0, "query.is_sleeping"),
        0.0
    );
    let sleeping = actor_with_metadata(HashMap::from([(
        92,
        ActorMetadataValue::FlagsExtended(1 << (76 - 64)),
    )]));
    assert_eq!(
        read(
            &sleeping,
            &ActorTickInput::default(),
            0,
            "query.is_sleeping"
        ),
        1.0
    );
}

#[test]
fn animation_reset_clock_is_distinct_from_actor_lifetime() {
    let actor = actor_with_metadata(HashMap::new());
    let input = ActorTickInput::default();
    assert_eq!(read(&actor, &input, 7, "query.anim_time"), 0.0);
    assert!((read(&actor, &input, 7, "query.life_time") - 0.35).abs() < 1.0e-6);
}

#[test]
fn riding_query_reads_only_the_authoritative_tick_input() {
    let actor = actor_with_metadata(HashMap::new());
    let mut input = ActorTickInput {
        is_riding: true,
        ..ActorTickInput::default()
    };
    assert_eq!(read(&actor, &input, 0, "query.is_riding"), 1.0);
    input.is_riding = false;
    assert_eq!(read(&actor, &input, 0, "query.is_riding"), 0.0);
}

#[test]
fn head_target_yaw_is_relative_to_the_body_and_wrapped() {
    let actor = actor_with_metadata(HashMap::new());
    let input = ActorTickInput {
        body_yaw: 170.0,
        head_yaw: -170.0,
        pitch: 30.0,
        ..ActorTickInput::default()
    };
    assert!((read(&actor, &input, 0, "query.target_y_rotation") - 20.0).abs() < 1.0e-4);
    assert_eq!(read(&actor, &input, 0, "query.target_x_rotation"), 30.0);
}

#[test]
fn operation_work_and_transition_budgets_are_aggregate() {
    let mut world_left = 1;
    let mut budget = EvalBudget {
        actor_left: 2,
        world_left: &mut world_left,
        work_left: 1,
        transitions_left: MAX_CONTROLLER_TRANSITIONS_PER_TICK,
        used: 0,
    };
    assert_eq!(budget.charge(), Ok(()));
    assert_eq!(budget.charge(), Err(EvalError::WorldBudget));
    assert_eq!(budget.charge_work(), Ok(()));
    assert_eq!(budget.charge_work(), Err(EvalError::ActorBudget));
    for _ in 0..MAX_CONTROLLER_TRANSITIONS_PER_TICK {
        assert!(budget.take_transition());
    }
    assert!(!budget.take_transition());
}
