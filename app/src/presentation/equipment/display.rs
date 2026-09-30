//! Composition of an item's display placement with the hand bone's pose.

use bevy::math::{Mat3, Mat4, Quat, Vec3};
use render::RenderBoneTransform;

pub(super) const LAYER_MAIN_HAND: u8 = 1;
pub(super) const LAYER_OFF_HAND: u8 = 2;
pub(super) const LAYER_HELMET: u8 = 3;
pub(super) const LAYER_CHESTPLATE: u8 = 4;
pub(super) const LAYER_LEGGINGS: u8 = 5;
pub(super) const LAYER_BOOTS: u8 = 6;

/// Item-space to hand-bone placement: rotation, translation in blocks and uniform scale.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct ItemDisplay {
    pub(super) rotation: Quat,
    pub(super) translation: Vec3,
    pub(super) scale: f32,
}

impl ItemDisplay {
    /// Decomposes a rotation + uniform scale + translation matrix.
    fn from_matrix(matrix: Mat4) -> Self {
        let linear = Mat3::from_mat4(matrix);
        let scale = linear.determinant().cbrt();
        Self {
            rotation: Quat::from_mat3(&(linear * scale.recip())).normalize(),
            translation: matrix.w_axis.truncate(),
            scale,
        }
    }
}

fn degrees(value: f32) -> f32 {
    value.to_radians()
}

/// The reference's hand-bone frame (Y and X negated) turned into the rig bone frame.
fn rig_from_reference_bone() -> Mat4 {
    Mat4::from_scale(Vec3::new(-1.0, -1.0, 1.0))
}

/// `ItemInHandRenderer::_applyDefaultItemTransforms` for a flat sprite in hand: the 1.5 scale
/// and tilt that seat vanilla's held-sprite mesh (`held_sprite_vertices`) in the grip.
fn item_default() -> Mat4 {
    Mat4::from_scale(Vec3::splat(1.5))
        * Mat4::from_rotation_y(degrees(50.0))
        * Mat4::from_rotation_z(degrees(335.0))
        * Mat4::from_translation(Vec3::new(0.075, -0.245, -0.1))
}

/// Third-person main-hand placement of a flat sprite item on the `rightItem` bone, from the
/// 26.30 reference's held-item and default item transforms. `hand_equipped` items (tools,
/// weapons, rods) are held upright like a sword.
pub(super) fn held_sprite_display(hand_equipped: bool) -> ItemDisplay {
    let grip = if hand_equipped {
        Mat4::from_rotation_y(degrees(180.0))
            * Mat4::from_translation(Vec3::new(0.1, 0.265, 0.0))
            * Mat4::from_scale(Vec3::splat(0.625))
            * Mat4::from_rotation_x(degrees(80.0))
            * Mat4::from_rotation_y(degrees(45.0))
    } else {
        Mat4::from_translation(Vec3::new(0.3125, 0.1875, -0.1875))
            * Mat4::from_scale(Vec3::splat(0.375))
            * Mat4::from_rotation_z(degrees(60.0))
            * Mat4::from_rotation_x(degrees(-90.0))
            * Mat4::from_rotation_z(degrees(20.0))
    };
    ItemDisplay::from_matrix(rig_from_reference_bone() * grip * item_default())
}

/// The item's single bone: the hand bone's pose with `display` applied in the hand frame, so
/// item-space vertices (bind pivot at the origin) land where the hand holds them. `None` for a
/// non-finite pose.
pub(super) fn attach_to_bone(
    hand: RenderBoneTransform,
    display: ItemDisplay,
) -> Option<RenderBoneTransform> {
    let [rx, ry, rz, rw] = hand.rotation;
    let hand_rotation = Quat::from_vec4(bevy::math::Vec4::new(rx, ry, rz, rw).try_normalize()?);
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

/// Main-hand placement of a block item's centred unit cube, from the reference's legacy block
/// item transforms (its block mesh origin is assumed centred and needs confirming).
pub(super) fn held_block_display() -> ItemDisplay {
    ItemDisplay::from_matrix(
        rig_from_reference_bone()
            * Mat4::from_translation(Vec3::new(0.0, 0.1875, -0.3125))
            * Mat4::from_rotation_x(degrees(200.0))
            * Mat4::from_rotation_y(degrees(225.0))
            * Mat4::from_scale(Vec3::splat(0.375)),
    )
}

/// A block worn on the head: a cube just larger than the head, centred on it. Provisional.
pub(super) fn head_block_display() -> ItemDisplay {
    ItemDisplay {
        rotation: Quat::IDENTITY,
        translation: Vec3::new(0.0, 0.25, 0.0),
        scale: 0.5625,
    }
}

/// Items vanilla holds upright (its `isHandEquipped`): tools, weapons and rod-like items. The
/// reference keeps this per item in code; the list mirrors vanilla's hand-equipped items.
pub(super) fn is_hand_equipped(identifier: &str) -> bool {
    let name = identifier.strip_prefix("minecraft:").unwrap_or(identifier);
    ["_sword", "_axe", "_pickaxe", "_shovel", "_hoe", "_spear"]
        .iter()
        .any(|suffix| name.ends_with(suffix))
        || matches!(
            name,
            "stick"
                | "bone"
                | "blaze_rod"
                | "breeze_rod"
                | "fishing_rod"
                | "carrot_on_a_stick"
                | "warped_fungus_on_a_stick"
                | "mace"
                | "debug_stick"
        )
}
