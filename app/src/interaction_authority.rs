//! Immutable pose, ray, stack and clicked-target authority shared by interactions.
//! The existing collision ray is a bounded provisional interaction slice, not
//! complete pickable-shape or camera-origin parity.

use std::num::NonZeroU64;

use protocol::PlayerInputMode;
use sim::{PaletteWorld, Vec3};

use crate::{
    local_player::InteractionOriginSnapshot,
    mining::{FrozenMiningFrame, FrozenMiningRay, FrozenMiningSelection, FrozenMiningTarget},
    movement::PhysicsCollisionRegistries,
    runtime::world::ClientWorld,
    ui_runtime::UiRuntime,
};

// Bound a retained edge across render frames without extending it through a
// stalled simulation. This preserves the existing Creative attachment limit.
pub(super) const MAX_PENDING_INTERACTION_FRAMES: u64 = 32;

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct FrozenBlockObservation {
    pub(crate) frame: FrozenMiningFrame,
    pub(crate) ray: FrozenMiningRay,
    pub(crate) reach: f64,
    pub(crate) input_mode: PlayerInputMode,
    pub(crate) selection: FrozenMiningSelection,
    pub(crate) target: FrozenMiningTarget,
}

type ObservationParts<'a> = (
    &'a FrozenMiningFrame,
    &'a FrozenMiningRay,
    f64,
    PlayerInputMode,
    &'a FrozenMiningSelection,
    &'a FrozenMiningTarget,
);

pub(crate) fn still_authorized_by(
    previous: ObservationParts<'_>,
    current: ObservationParts<'_>,
) -> bool {
    let (frame, ray, reach, input_mode, selection, target) = previous;
    let (next_frame, next_ray, next_reach, next_mode, next_selection, next_target) = current;
    frame.session_generation == next_frame.session_generation
        && frame.position_authority_generation == next_frame.position_authority_generation
        && frame.input_authority_generation == next_frame.input_authority_generation
        && frame.input_frame_sequence <= next_frame.input_frame_sequence
        && next_frame
            .input_frame_sequence
            .saturating_sub(frame.input_frame_sequence)
            <= MAX_PENDING_INTERACTION_FRAMES
        && frame.fifo_sequence <= next_frame.fifo_sequence
        && frame.physics_tick <= next_frame.physics_tick
        && frame.pose_generation <= next_frame.pose_generation
        && ray.origin.into_iter().all(f32::is_finite)
        && ray.direction.into_iter().all(f32::is_finite)
        && ray.movement_world_identity == next_ray.movement_world_identity
        && ray.world_identity == next_ray.world_identity
        && reach == next_reach
        && input_mode == next_mode
        && selection == next_selection
        && target.position == next_target.position
        && target.face == next_target.face
        && target.runtime_id == next_target.runtime_id
        && target.identity == next_target.identity
}

/// Server-side pick checks measure to the block's minimum corner with this slack
/// over the game-mode pick range. Needs independent measurement.
const SERVER_PICK_SLACK: f64 = 0.5;

/// Vanilla limits a pick by the eye-to-block-centre distance, not the ray length.
///
/// Only touch reach (6.7 survival, 12 creative) equals the server's range, so only
/// touch picks can exceed its corner check; those are dropped too.
pub(crate) fn within_pick_range(observed: &FrozenBlockObservation) -> bool {
    let distance_squared = |offset: f64| {
        observed
            .target
            .position
            .into_iter()
            .zip(observed.ray.origin)
            .map(|(block, eye)| (f64::from(block) + offset - f64::from(eye)).powi(2))
            .sum::<f64>()
    };
    let corner_limit = observed.reach + SERVER_PICK_SLACK;
    distance_squared(0.5) <= observed.reach * observed.reach
        && (observed.input_mode != PlayerInputMode::Touch
            || distance_squared(0.0) <= corner_limit * corner_limit)
}

#[cfg(test)]
impl FrozenBlockObservation {
    /// A top-face hit on `position` holding `item` in slot 2.
    pub(crate) fn fixture(
        position: [i32; 3],
        face: u8,
        item: protocol::VerifiedNetworkItemStack,
    ) -> Self {
        let identity = sim::CollisionQuery::synthetic(()).identity;
        Self {
            frame: FrozenMiningFrame {
                session_generation: 7,
                position_authority_generation: 0,
                input_authority_generation: NonZeroU64::MIN,
                input_frame_sequence: 1,
                fifo_sequence: 1,
                physics_tick: 101,
                pose_generation: 1,
            },
            ray: FrozenMiningRay {
                origin: [0.5, 65.62, 0.5],
                direction: [0.0, -1.0, 0.0],
                movement_world_identity: identity.clone(),
                world_identity: identity.clone(),
            },
            reach: 5.7,
            input_mode: PlayerInputMode::Mouse,
            selection: FrozenMiningSelection { slot: 2, item },
            target: FrozenMiningTarget {
                position,
                face,
                relative_hit: [0.5, 1.0, 0.5],
                runtime_id: 9,
                identity,
            },
        }
    }
}

/// The ray or world evidence behind a block observation is stale or unreadable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct BlockRayUnavailable;

pub(crate) fn observe_block(
    origin: &InteractionOriginSnapshot,
    ui: &UiRuntime,
    client_world: &ClientWorld,
    collisions: &PhysicsCollisionRegistries,
    selection: FrozenMiningSelection,
    input: (PlayerInputMode, f64, (NonZeroU64, u64), u64),
) -> Option<FrozenBlockObservation> {
    observe_block_ray(origin, ui, client_world, collisions, selection, input)
        .ok()
        .flatten()
}

/// The nearest block on the current ray; `Ok(None)` only for a verified clear ray.
pub(crate) fn observe_block_ray(
    origin: &InteractionOriginSnapshot,
    ui: &UiRuntime,
    client_world: &ClientWorld,
    collisions: &PhysicsCollisionRegistries,
    selection: FrozenMiningSelection,
    input: (PlayerInputMode, f64, (NonZeroU64, u64), u64),
) -> Result<Option<FrozenBlockObservation>, BlockRayUnavailable> {
    let (
        input_mode,
        reach,
        (input_authority_generation, input_frame_sequence),
        position_authority_generation,
    ) = input;
    let ray = origin.outbound_ray().ok_or(BlockRayUnavailable)?;
    let stream = client_world.stream.as_ref().ok_or(BlockRayUnavailable)?;
    if ray.session_generation() != ui.session_id()
        || ray.session_generation() != stream.actor_session_id()
        || stream.committed_sequence() != ray.fifo_sequence()
    {
        return Err(BlockRayUnavailable);
    }
    let vector = |value: bevy::prelude::Vec3| {
        Vec3::new(f64::from(value.x), f64::from(value.y), f64::from(value.z))
    };
    let world = PaletteWorld::new(
        stream.collision_store(),
        collisions.registry(stream.network_id_mode()),
        stream.current_dimension(),
    );
    let Some(hit) = world
        .block_interaction_ray_current(vector(ray.origin()), vector(ray.direction()), reach)
        .map_err(|_| BlockRayUnavailable)?
    else {
        return Ok(None);
    };
    Ok(Some(FrozenBlockObservation {
        frame: FrozenMiningFrame {
            session_generation: ray.session_generation(),
            position_authority_generation,
            input_authority_generation,
            input_frame_sequence,
            fifo_sequence: ray.fifo_sequence(),
            physics_tick: ray.physics_tick(),
            pose_generation: ray.pose_generation(),
        },
        ray: FrozenMiningRay {
            origin: ray.origin().to_array(),
            direction: ray.direction().to_array(),
            movement_world_identity: ray.world_collision_identity().clone(),
            world_identity: hit.identity.clone(),
        },
        reach,
        input_mode,
        selection,
        target: FrozenMiningTarget {
            position: hit.block_pos,
            face: hit.face,
            relative_hit: [
                hit.hit_local.x as f32,
                hit.hit_local.y as f32,
                hit.hit_local.z as f32,
            ],
            runtime_id: hit.runtime_id,
            identity: hit.identity,
        },
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(position: [i32; 3], input_mode: PlayerInputMode, reach: f64) -> FrozenBlockObservation {
        let stack = protocol::NetworkItemStack::empty();
        let item =
            protocol::VerifiedNetworkItemStack::try_new(stack.clone(), stack.nbt_digest).unwrap();
        FrozenBlockObservation {
            input_mode,
            reach,
            ..FrozenBlockObservation::fixture(position, 1, item)
        }
    }

    #[test]
    fn touch_picks_past_the_server_corner_limit_are_dropped() {
        // Eye at (0.5, 65.62, 0.5); a block down the negative axes is centre-near, corner-far.
        let far_corner = [-5, 62, -3];
        assert!(!within_pick_range(&at(
            far_corner,
            PlayerInputMode::Touch,
            6.7
        )));
        assert!(within_pick_range(&at(
            [-4, 63, -2],
            PlayerInputMode::Touch,
            6.7
        )));
        // Mouse reach cannot reach the corner limit, so only the centre rule applies.
        assert!(within_pick_range(&at(
            [-4, 63, -1],
            PlayerInputMode::Mouse,
            5.7
        )));
        assert!(!within_pick_range(&at(
            [-6, 63, 0],
            PlayerInputMode::Mouse,
            5.7
        )));
    }
}
