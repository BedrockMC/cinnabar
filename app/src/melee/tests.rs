use std::collections::HashMap;

use client_world::{ActorPose, ActorSnapshot};
use protocol::{ActorKind, ActorMetadataValue};

use super::*;

fn actor(
    runtime_id: u64,
    identifier: &str,
    feet: [f32; 3],
    size: Option<(f32, f32)>,
) -> ActorSnapshot {
    let pose = ActorPose {
        position: feet,
        pitch: 0.0,
        yaw: 0.0,
        head_yaw: 0.0,
    };
    let metadata = size
        .map(|(width, height)| {
            HashMap::from([
                (53, ActorMetadataValue::Float(width)),
                (54, ActorMetadataValue::Float(height)),
            ])
        })
        .unwrap_or_default();
    ActorSnapshot {
        unique_id: runtime_id as i64 * 10,
        runtime_id,
        spawn_revision: 0,
        movement_revision: 0,
        kind: ActorKind::Entity {
            identifier: identifier.into(),
        },
        position: feet,
        velocity: [0.0; 3],
        pitch: 0.0,
        yaw: 0.0,
        head_yaw: 0.0,
        previous_pose: pose,
        received_pose: pose,
        interpolation_ticks_remaining: 0,
        body_yaw: 0.0,
        on_ground: None,
        teleported: false,
        player_mode: None,
        source_tick: None,
        metadata,
        attributes: HashMap::new(),
        int_properties: HashMap::new(),
        float_properties: HashMap::new(),
    }
}

const EYE: [f32; 3] = [0.0, 1.62, 0.0];
const NORTH: [f32; 3] = [0.0, 0.0, -1.0];

#[test]
fn nearest_pickable_actor_wins_and_reports_the_inflated_entry_point() {
    let near = actor(1, "minecraft:zombie", [0.0, 0.0, -2.0], Some((0.6, 1.95)));
    let far = actor(2, "minecraft:zombie", [0.0, 0.0, -4.0], Some((0.6, 1.95)));
    let drop = actor(3, "minecraft:item", [0.0, 0.0, -1.0], Some((0.25, 0.25)));
    let sizeless = actor(4, "minecraft:cow", [0.0, 0.0, -1.5], None);
    let actors = [far, drop, sizeless, near];
    let hit = pick_actor(actors.iter(), None, EYE, NORTH, 5.7).unwrap();
    assert_eq!(hit.runtime_id, 1);
    // Front face at z = -2 + 0.3, grown by the pick radius.
    assert!((hit.distance - 1.6).abs() < 1e-6, "{}", hit.distance);
    assert!((hit.point[2] + 1.6).abs() < 1e-6);
    // The ridden vehicle is never picked.
    let hit = pick_actor(actors.iter(), Some(10), EYE, NORTH, 5.7).unwrap();
    assert_eq!(hit.runtime_id, 2);
    assert_eq!(pick_actor(actors[..1].iter(), None, EYE, NORTH, 3.0), None);
}

#[test]
fn an_eye_inside_the_box_hits_at_zero_distance() {
    let around = actor(5, "minecraft:slime", [0.0, 0.0, 0.0], Some((2.0, 2.0)));
    let hit = pick_actor([around].iter(), None, EYE, [1.0, 0.0, 0.0], 3.0).unwrap();
    assert_eq!(hit.distance, 0.0);
    assert_eq!(pick_actor([].iter(), None, EYE, [0.0; 3], 3.0), None);
}

#[test]
fn survival_reach_and_block_occlusion_decide_the_press() {
    let hit = |distance| ActorHit {
        runtime_id: 7,
        distance,
        point: [0.0; 3],
    };
    assert_eq!(
        classify(Some(hit(2.0)), Some(4.0), 3.0),
        Crosshair::Actor(hit(2.0))
    );
    // In front of the block but beyond melee reach: nothing is targeted.
    assert_eq!(classify(Some(hit(3.5)), Some(5.0), 3.0), Crosshair::Miss);
    assert_eq!(classify(Some(hit(3.5)), None, 3.0), Crosshair::Miss);
    // The actor must beat the block by the pick radius.
    assert_eq!(classify(Some(hit(1.95)), Some(2.0), 3.0), Crosshair::Block);
    assert_eq!(classify(None, Some(2.0), 3.0), Crosshair::Block);
    assert_eq!(classify(None, None, 3.0), Crosshair::Miss);
    assert_eq!(
        classify(Some(hit(5.0)), None, 7.0),
        Crosshair::Actor(hit(5.0))
    );
}

#[test]
fn a_new_swing_waits_for_half_the_current_one() {
    let mut swings = SwingTracker::default();
    assert!(swings.try_swing(10, 6));
    assert!(!swings.try_swing(10, 6));
    assert!(!swings.try_swing(12, 6));
    assert!(swings.try_swing(13, 6));
    // Reanchored tick numbers never lock swinging out.
    assert!(swings.try_swing(2, 6));
}

#[test]
fn haste_shortens_and_fatigue_lengthens_the_swing() {
    let effects = |haste, fatigue| MiningEffects {
        haste,
        mining_fatigue: fatigue,
        conduit_power: None,
    };
    assert_eq!(swing_duration(effects(None, None)), 6);
    assert_eq!(swing_duration(effects(Some(0), None)), 5);
    assert_eq!(swing_duration(effects(None, Some(1))), 10);
    assert_eq!(swing_duration(effects(Some(1), Some(1))), 4);
    assert_eq!(swing_duration(effects(Some(40), None)), 1);
}
