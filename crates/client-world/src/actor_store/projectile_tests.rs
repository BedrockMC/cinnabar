use super::*;

/// Spawns an arrow with its first motion arriving separately from its position.
fn arrow_store() -> ActorStore {
    let mut store = ActorStore::new(1, 0);
    store.apply(
        1,
        1,
        ActorEvent::Spawn(ActorSpawnEvent {
            dimension: 0,
            unique_id: 77,
            runtime_id: 77,
            kind: ActorKind::Entity {
                identifier: "minecraft:arrow".into(),
            },
            position: [1.0, 64.0, 2.0],
            velocity: [0.0; 3],
            pitch: 0.0,
            yaw: 0.0,
            head_yaw: 0.0,
            body_yaw: 0.0,
            held_item: Default::default(),
            metadata: std::sync::Arc::from([]),
            attributes: std::sync::Arc::from([]),
            properties: std::sync::Arc::from([]),
            links: std::sync::Arc::from([]),
        }),
    );
    store
}

#[test]
fn first_arrow_motion_initializes_rotation_without_moving_the_actor() {
    let mut store = arrow_store();
    store.apply_motion(
        2,
        protocol::ActorMotionEvent {
            actor_runtime_id: 77,
            motion: [1.0, 1.0, 0.0],
            tick: 1,
        },
    );
    let actor = store.get(77).unwrap();
    assert_eq!(actor.yaw, 90.0);
    assert_eq!(actor.pitch, 45.0);
    assert_eq!(actor.previous_pose.yaw, actor.yaw);
    assert_eq!(actor.previous_pose.pitch, actor.pitch);
    assert_eq!(actor.position, [1.0, 64.0, 2.0]);
    store.advance_interpolation_ticks(1);
    let actor = store.get(77).unwrap();
    assert_eq!(actor.yaw, 90.0);
    assert_eq!(actor.pitch, 45.0);
}

#[test]
fn later_arrow_motion_keeps_the_existing_rotation() {
    let mut store = arrow_store();
    store.apply_motion(
        2,
        protocol::ActorMotionEvent {
            actor_runtime_id: 77,
            motion: [1.0, 1.0, 0.0],
            tick: 1,
        },
    );
    store.apply_motion(
        3,
        protocol::ActorMotionEvent {
            actor_runtime_id: 77,
            motion: [-1.0, -1.0, 0.0],
            tick: 2,
        },
    );
    let actor = store.get(77).unwrap();
    assert_eq!((actor.yaw, actor.pitch), (90.0, 45.0));
    assert_eq!(actor.velocity, [-1.0, -1.0, 0.0]);
}
