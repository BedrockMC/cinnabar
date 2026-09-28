use serde::{Deserialize, Serialize};

use crate::{Aabb, CollisionWorld, PLAYER_HEIGHT, Vec3, WorldQueryError};

/// Sneaking hitbox height. Public wiki value, not oracle-validated.
const SNEAK_HEIGHT: f64 = 1.5;
/// Swimming, crawling and gliding hitbox height. Public wiki value.
const LOW_POSE_HEIGHT: f64 = 0.6;

/// Locomotion mode the client selected for one tick; the simulator never picks it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MovementMode {
    #[default]
    Walking,
    /// Pose-swimming: travel follows the look direction through water.
    Swimming,
    /// Land movement in a 0.6-high box, at sneak speed.
    Crawling,
    Gliding,
    /// Ability flight: no gravity, vertical motion from jump and sneak.
    Flying,
}

impl MovementMode {
    #[must_use]
    pub fn hitbox_height(self, sneaking: bool) -> f64 {
        match self {
            Self::Swimming | Self::Crawling | Self::Gliding => LOW_POSE_HEIGHT,
            Self::Walking if sneaking => SNEAK_HEIGHT,
            Self::Walking | Self::Flying => PLAYER_HEIGHT,
        }
    }

    #[must_use]
    pub const fn is_walking(self) -> bool {
        matches!(self, Self::Walking)
    }
}

/// Whether the player box for `mode` fits at `feet` without touching a solid.
pub fn pose_fits(
    world: &impl CollisionWorld,
    feet: Vec3,
    mode: MovementMode,
    sneaking: bool,
) -> Result<bool, WorldQueryError> {
    let query = Aabb::player_with_height_at(feet, mode.hitbox_height(sneaking));
    crate::world::validate_collision_query(query)?;
    let boxes = world.collision_boxes(query)?;
    Ok(!boxes.value.into_iter().any(|shape| shape.intersects(query)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn low_poses_share_one_height_and_sneak_only_shrinks_walking() {
        assert_eq!(MovementMode::Swimming.hitbox_height(false), 0.6);
        assert_eq!(MovementMode::Crawling.hitbox_height(true), 0.6);
        assert_eq!(MovementMode::Gliding.hitbox_height(false), 0.6);
        assert_eq!(MovementMode::Walking.hitbox_height(true), 1.5);
        assert_eq!(MovementMode::Walking.hitbox_height(false), PLAYER_HEIGHT);
        assert_eq!(MovementMode::Flying.hitbox_height(true), PLAYER_HEIGHT);
    }
}
