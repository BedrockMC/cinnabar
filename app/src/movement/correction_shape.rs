//! Correction-shape classification for committed local-player corrections.
//!
//! Protocol 2168's `CorrectPlayerMovePrediction` carries no shape or mode
//! field, so Cinnabar derives the handling shape from observable field
//! combinations. Every threshold here is explicit client policy and is labeled
//! provisional until a version-matched native reference measures real
//! correction behavior; none of this claims a vanilla contract.

use protocol::PLAYER_NETWORK_OFFSET;
use sim::CollisionWorld;

use super::physics::LocalPhysicsController;
use super::{
    MovementTicker, PhysicsAnchor, PhysicsAuthorityFault, PhysicsCorrectionMode,
    PhysicsCorrectionOutcome, reconcile_physics_anchor,
};

/// Largest per-tick displacement still treated as an ordinary reconcilable
/// correction.
///
/// One full chunk column (16 blocks) within a single 20 Hz tick exceeds every
/// vanilla Bedrock locomotion ceiling — terminal fall speed is roughly 3.9
/// blocks per tick and sprint jumping stays far below one block per tick — so a
/// larger server displacement cannot be reproduced by replaying retained inputs
/// and is handled through the existing teleport anchor path instead.
/// Provisional policy pending version-matched native measurement.
pub const CORRECTION_TELEPORT_DISPLACEMENT_BLOCKS: f32 = 16.0;

/// How one committed correction must be applied to prediction state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CorrectionShape {
    /// Position, motion and ground flag match the retained frame within the
    /// vanilla epsilon, so nothing is replayed and no state is touched.
    Confirmed,
    /// Ordinary small or full correction reconciled by replacing the retained
    /// position/ground at its tick and replaying later inputs. This is today's
    /// established behavior and also covers any correction that is not exactly
    /// confirming and stays within the teleport displacement bound.
    Replay,
    /// Displacement beyond [`CORRECTION_TELEPORT_DISPLACEMENT_BLOCKS`], or an
    /// unresolvable non-finite anchor: snap through the existing teleport
    /// anchor path, including its bounded state clearing and settle window.
    TeleportSnap,
}

/// Squared distance within which vanilla treats a correction's position and
/// motion as already matching the retained frame (`getAdvanceFrameResult`).
const CORRECTION_MATCH_EPSILON_SQUARED: f32 = 1.0e-5;

impl LocalPhysicsController {
    /// Classifies one committed correction against prediction state retained
    /// for the correction's own authoritative tick.
    ///
    /// Matching position, motion (when carried) and ground flag within the
    /// vanilla epsilon needs no replay. A missing retained tick selects replay
    /// so the not-retained policy decides instead of unrelated current state.
    #[must_use]
    pub fn correction_shape(
        &self,
        network_position: [f32; 3],
        correction_tick: u64,
        on_ground: bool,
        velocity: Option<[f32; 3]>,
    ) -> CorrectionShape {
        if !network_position.into_iter().all(f32::is_finite) {
            // Position resolution bounds non-finite input upstream, so this is
            // pure defense: an unresolvable anchor is rejected by the
            // controller's InvalidAnchor guard before any shape-specific path
            // runs, leaving prediction state untouched.
            return CorrectionShape::TeleportSnap;
        }
        let Some(state) = self.retained_state(correction_tick) else {
            return CorrectionShape::Replay;
        };
        let current = [
            state.position.x as f32,
            state.position.y as f32 + PLAYER_NETWORK_OFFSET,
            state.position.z as f32,
        ];
        let position_error = squared_distance(current, network_position);
        let velocity_matches = velocity.is_none_or(|velocity| {
            let retained = [
                state.velocity.x as f32,
                state.velocity.y as f32,
                state.velocity.z as f32,
            ];
            squared_distance(retained, velocity) <= CORRECTION_MATCH_EPSILON_SQUARED
        });
        if position_error <= CORRECTION_MATCH_EPSILON_SQUARED
            && velocity_matches
            && state.on_ground == on_ground
        {
            return CorrectionShape::Confirmed;
        }
        let bound = CORRECTION_TELEPORT_DISPLACEMENT_BLOCKS;
        if position_error > bound * bound {
            CorrectionShape::TeleportSnap
        } else {
            CorrectionShape::Replay
        }
    }
}

fn squared_distance(a: [f32; 3], b: [f32; 3]) -> f32 {
    let dx = a[0] - b[0];
    let dy = a[1] - b[1];
    let dz = a[2] - b[2];
    dx * dx + dy * dy + dz * dz
}

/// Applies one committed correction to prediction according to its shape.
///
/// `Ok(None)` means the correction confirmed the current prediction and
/// deliberately mutated nothing — no replay, no interpolation re-anchor, no
/// settle-window engagement. `Ok(Some(_))` reports the applied outcome for
/// evidence attribution.
pub(crate) fn reconcile_committed_correction(
    ticker: &mut MovementTicker,
    physics: &mut LocalPhysicsController,
    network_position: [f32; 3],
    correction_tick: u64,
    on_ground: bool,
    velocity: Option<[f32; 3]>,
    world: &impl CollisionWorld,
) -> Result<Option<PhysicsCorrectionOutcome>, PhysicsAuthorityFault> {
    let shape = physics.correction_shape(network_position, correction_tick, on_ground, velocity);
    if shape != CorrectionShape::Confirmed {
        super::diagnostics::note_correction(
            super::diagnostics::CorrectionKind::Correct,
            correction_tick,
            network_position,
            on_ground,
            physics.sample_at(correction_tick),
        );
    }
    let mode = match shape {
        CorrectionShape::Confirmed => return Ok(None),
        CorrectionShape::Replay => PhysicsCorrectionMode::ReplayIfRetained,
        CorrectionShape::TeleportSnap => PhysicsCorrectionMode::Snap,
    };
    reconcile_physics_anchor(
        ticker,
        physics,
        PhysicsAnchor {
            network_position,
            tick: correction_tick,
            on_ground,
            velocity,
        },
        mode,
        world,
    )
    .map(Some)
}
