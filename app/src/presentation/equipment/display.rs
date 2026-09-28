//! Composition of an item's display placement with the hand bone's pose.

use bevy::math::{Quat, Vec3};
use render::RenderBoneTransform;

pub(super) const LAYER_MAIN_HAND: u8 = 1;
pub(super) const LAYER_OFF_HAND: u8 = 2;
pub(super) const LAYER_HELMET: u8 = 3;
pub(super) const LAYER_CHESTPLATE: u8 = 4;
pub(super) const LAYER_LEGGINGS: u8 = 5;
pub(super) const LAYER_BOOTS: u8 = 6;

/// Item-space to hand-bone placement: rotation, translation in blocks, uniform scale.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct ItemDisplay {
    pub(super) rotation: Quat,
    pub(super) translation: Vec3,
    pub(super) scale: f32,
}

/// Third-person placement of a flat sprite item held in the main hand.
///
/// Provisional: it turns the sprite's up axis to the hand's up and its width axis to forward so
/// the tip points forward-up, with the grip corner at the fist. Scale and offsets need native
/// measurement against the retail client; no vanilla number is derived here.
pub(super) fn held_sprite_display() -> ItemDisplay {
    const SCALE: f32 = 0.85;
    const GRIP_FROM_CENTER: f32 = 0.31;
    ItemDisplay {
        rotation: Quat::from_rotation_y(std::f32::consts::FRAC_PI_2),
        translation: Vec3::new(0.0, GRIP_FROM_CENTER * SCALE, -GRIP_FROM_CENTER * SCALE),
        scale: SCALE,
    }
}

/// The item's single bone: the hand bone's pose with `display` applied in the hand frame, so
/// item-space vertices (bind pivot at the origin) land where the hand holds them. `None` for a
/// non-finite pose.
pub(super) fn attach_to_bone(
    hand: RenderBoneTransform,
    display: ItemDisplay,
) -> Option<RenderBoneTransform> {
    let [rx, ry, rz, rw] = hand.rotation;
    let hand_rotation = Quat::from_xyzw(rx, ry, rz, rw).try_normalize()?;
    // A non-uniform hand scale would shear the item; the first axis stands in for it.
    let hand_scale = hand.translation_scale[3] * hand.axis_scale[0];
    let origin = Vec3::new(
        hand.translation_scale[0],
        hand.translation_scale[1],
        hand.translation_scale[2],
    );
    let rotation = (hand_rotation * display.rotation).normalize();
    let translation = origin + hand_rotation * (display.translation * hand_scale);
    let bone = RenderBoneTransform {
        rotation: rotation.to_array(),
        translation_scale: [
            translation.x,
            translation.y,
            translation.z,
            hand_scale * display.scale,
        ],
        axis_scale: render::UNIT_AXIS_SCALE,
    };
    bone.is_finite().then_some(bone)
}

/// Third-person placement of a held block cube: a small cube turned to show its corner, resting
/// in front of the fist. Provisional; needs native measurement like the sprite placement.
pub(super) fn held_block_display() -> ItemDisplay {
    ItemDisplay {
        rotation: Quat::from_rotation_y(std::f32::consts::FRAC_PI_4),
        translation: Vec3::new(0.0, 0.05, -0.2),
        scale: 0.4,
    }
}

/// A block worn on the head: a cube just larger than the head, centred on it. Provisional.
pub(super) fn head_block_display() -> ItemDisplay {
    ItemDisplay {
        rotation: Quat::IDENTITY,
        translation: Vec3::new(0.0, 0.25, 0.0),
        scale: 0.5625,
    }
}
