//! Sends `ClientMovementPredictionSync` after the server corrected the local player.

use std::{collections::HashMap, time::Duration};

use bevy::{
    prelude::{Local, Res},
    time::{Real, Time},
};
use protocol::{ActorMetadataValue, MovementPredictionSync, client_movement_prediction_sync};

use super::LocalPhysicsController;
use crate::runtime::{network::NetworkHandle, world::ClientWorld};

/// Provisional minimum spacing between syncs; the vanilla timer interval is unmeasured.
const MIN_SYNC_INTERVAL: Duration = Duration::from_secs(1);

const FLAGS_KEY: u32 = 0;
const EXTENDED_FLAGS_KEY: u32 = 92;
const SCALE_KEY: u32 = 38;
const WIDTH_KEY: u32 = 53;
const HEIGHT_KEY: u32 = 54;

/// Attribute names in wire order of the sync's attribute block; unset attributes read 0.
const ATTRIBUTE_NAMES: [&str; 6] = [
    "minecraft:movement",
    "minecraft:underwater_movement",
    "minecraft:lava_movement",
    "minecraft:horse.jump_strength",
    "minecraft:health",
    "minecraft:player.hunger",
];
/// Provisional values for the three movement modifiers no attribute update carries.
const DEFAULT_FRICTION_MODIFIER: f32 = 1.0;
const DEFAULT_BOUNCINESS: f32 = 0.0;
const DEFAULT_AIR_DRAG_MODIFIER: f32 = 1.0;
const PLAYER_WIDTH: f32 = 0.6;
const PLAYER_HEIGHT: f32 = 1.8;

#[derive(Default)]
pub(crate) struct PredictionSyncState {
    seen_corrections: u64,
    pending: bool,
    last_sent: Option<Duration>,
}

pub(crate) fn send_movement_prediction_sync(
    time: Res<Time<Real>>,
    physics: Res<LocalPhysicsController>,
    client_world: Res<ClientWorld>,
    network: Option<Res<NetworkHandle>>,
    mut state: Local<PredictionSyncState>,
) {
    let applied = physics.corrections_applied();
    if applied != state.seen_corrections {
        state.seen_corrections = applied;
        state.pending = true;
    }
    let now = time.elapsed();
    if !state.pending
        || state
            .last_sent
            .is_some_and(|last| now.saturating_sub(last) < MIN_SYNC_INTERVAL)
    {
        return;
    }
    let (Some(network), Some(stream)) = (network.as_deref(), client_world.stream.as_ref()) else {
        return;
    };
    let Some(actor) = stream.actor(stream.local_player_runtime_id()) else {
        return;
    };
    let sync = MovementPredictionSync {
        actor_flags: flag_words(&actor.metadata),
        bounding_box: bounding_box(&actor.metadata),
        attributes: attributes(|name| {
            actor
                .attributes
                .get(name)
                .map(|attribute| attribute.current)
        }),
        unique_id: stream.local_player_unique_id(),
        flying: physics.mode() == sim::MovementMode::Flying,
    };
    if network
        .send_movement_packet(client_movement_prediction_sync(sync))
        .is_ok()
    {
        state.pending = false;
        state.last_sent = Some(now);
    }
}

fn flag_words(metadata: &HashMap<u32, ActorMetadataValue>) -> [u64; 3] {
    let word = |key| match metadata.get(&key) {
        Some(ActorMetadataValue::Flags(bits) | ActorMetadataValue::FlagsExtended(bits)) => *bits,
        _ => 0,
    };
    [word(FLAGS_KEY), word(EXTENDED_FLAGS_KEY), 0]
}

fn bounding_box(metadata: &HashMap<u32, ActorMetadataValue>) -> [f32; 3] {
    let float = |key, default| match metadata.get(&key) {
        Some(ActorMetadataValue::Float(value)) if value.is_finite() => *value,
        _ => default,
    };
    [
        float(SCALE_KEY, 1.0),
        float(WIDTH_KEY, PLAYER_WIDTH),
        float(HEIGHT_KEY, PLAYER_HEIGHT),
    ]
}

fn attributes(current: impl Fn(&str) -> Option<f32>) -> [f32; 9] {
    let value = |index: usize| current(ATTRIBUTE_NAMES[index]).unwrap_or(0.0);
    [
        value(0),
        value(1),
        value(2),
        value(3),
        value(4),
        value(5),
        DEFAULT_FRICTION_MODIFIER,
        DEFAULT_BOUNCINESS,
        DEFAULT_AIR_DRAG_MODIFIER,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flags_read_both_words_and_missing_ones_are_zero() {
        let mut metadata = HashMap::new();
        assert_eq!(flag_words(&metadata), [0; 3]);
        metadata.insert(FLAGS_KEY, ActorMetadataValue::Flags(0b1010));
        metadata.insert(EXTENDED_FLAGS_KEY, ActorMetadataValue::FlagsExtended(1));
        assert_eq!(flag_words(&metadata), [0b1010, 1, 0]);
    }

    #[test]
    fn bounding_box_falls_back_to_the_player_box() {
        let mut metadata = HashMap::new();
        assert_eq!(bounding_box(&metadata), [1.0, 0.6, 1.8]);
        metadata.insert(HEIGHT_KEY, ActorMetadataValue::Float(0.6));
        metadata.insert(WIDTH_KEY, ActorMetadataValue::Float(f32::NAN));
        assert_eq!(bounding_box(&metadata), [1.0, 0.6, 0.6]);
    }

    #[test]
    fn unset_attributes_read_zero_and_modifiers_use_their_defaults() {
        let block = attributes(|name| (name == "minecraft:movement").then_some(0.1));
        assert_eq!(block[0], 0.1);
        assert_eq!(&block[1..6], &[0.0; 5]);
        assert_eq!(&block[6..], &[1.0, 0.0, 1.0]);
    }
}
