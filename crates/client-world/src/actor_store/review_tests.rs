use super::*;
use std::sync::Arc;

#[test]
fn review_lead_holder_resolves_negative_unique_ids() {
    let mut store = ActorStore::new(1, 0);
    store.apply(1, 1, tests::spawn(42, -7));
    let ActorEvent::Spawn(mut mob) = tests::spawn(43, -8) else {
        unreachable!()
    };
    mob.metadata = Arc::from([protocol::ActorMetadata {
        key: 37,
        value: protocol::ActorMetadataValue::Long(-7),
    }]);
    store.apply(1, 2, ActorEvent::Spawn(mob));
    assert_eq!(store.ropes(0.0).len(), 1);
}

#[test]
fn review_nonfinite_movement_retains_valid_components_of_the_previous_pose() {
    let mut store = ActorStore::new(1, 0);
    store.apply(1, 1, tests::spawn(42, -7));
    let ActorEvent::Move(mut movement) = tests::player_move(42, f32::NAN, true) else {
        unreachable!()
    };
    movement.position[2] = Some(9.0);
    movement.pitch = Some(f32::INFINITY);
    movement.yaw = Some(45.0);
    movement.head_yaw = Some(f32::NEG_INFINITY);
    store.apply(1, 2, ActorEvent::Move(movement));
    assert_eq!(store.ignored_movement_components, 3);
    let pose = store.get(42).unwrap().received_pose;
    assert_eq!(pose.position, [1.0, 2.0, 9.0]);
    assert_eq!((pose.pitch, pose.yaw, pose.head_yaw), (0.0, 45.0, 0.0));
}
