//! Single-bone attachable geometry (trident, shield) placed at the hand item bone.
//!
//! The attachable's model origin is the hand item bone's origin and its own axes follow that
//! bone; each bone turns about its authored pivot by its literal offset, rotation, and scale.

use bevy::math::{Quat, Vec3};
use render::RenderBoneTransform;

use super::elytra::rotation;

/// A bone's literal channels: offset in pixels, rotation in degrees, per-axis scale.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct BoneChannels {
    pub(super) translation: [f32; 3],
    pub(super) rotation: [f32; 3],
    pub(super) scale: [f32; 3],
}

impl Default for BoneChannels {
    fn default() -> Self {
        Self {
            translation: [0.0; 3],
            rotation: [0.0; 3],
            scale: [1.0; 3],
        }
    }
}

/// The bone's pose in the hand item bone's frame: `pivot` is the authored bind pivot (rig frame,
/// blocks), and the offset mirrors authored X like the actor pose evaluator.
pub(super) fn attach(
    hand: RenderBoneTransform,
    pivot: [f32; 3],
    channels: BoneChannels,
) -> Option<RenderBoneTransform> {
    let [rx, ry, rz, rw] = hand.rotation;
    let hand_rotation = Quat::from_xyzw(rx, ry, rz, rw).try_normalize()?;
    let hand_scale = hand.translation_scale[3] * hand.axis_scale[0];
    let [x, y, z] = channels.translation;
    let offset = Vec3::new(-x, y, z) / 16.0;
    let origin = Vec3::new(
        hand.translation_scale[0],
        hand.translation_scale[1],
        hand.translation_scale[2],
    ) + hand_rotation * ((Vec3::from_array(pivot) + offset) * hand_scale);
    let turned = (hand_rotation * rotation(channels.rotation)).normalize();
    let [sx, sy, sz] = channels.scale;
    let bone = RenderBoneTransform {
        rotation: turned.to_array(),
        translation_scale: [origin.x, origin.y, origin.z, hand_scale],
        axis_scale: [sx, sy, sz, 1.0],
    };
    bone.is_finite().then_some(bone)
}
