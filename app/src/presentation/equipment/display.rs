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

/// How the first-person pass lays out a held item.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum FirstPersonShape {
    /// A flat sprite; `mirrored_art` items (rods on a stick) turn half a revolution.
    Sprite { mirrored_art: bool },
    /// A block's centred unit cube.
    Block,
}

/// The arm's state the first-person item follows, interpolated to the frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct FirstPersonHand {
    /// 0..1 swing progress.
    pub(crate) swing: f32,
    /// 0..1 equip progress.
    pub(crate) equip: f32,
    /// Ticks into an eat or drink use and its duration, while one runs.
    pub(crate) consume: Option<(f32, f32)>,
}

/// Camera-space placement of the first-person held item, from `renderFirstPerson`'s own item
/// transforms: the swing offset (or the eat/drink raise), the equip dip, the swing turns and
/// the 0.4 hand scale, then the item's default transforms.
pub(super) fn first_person_display(shape: FirstPersonShape, hand: FirstPersonHand) -> ItemDisplay {
    use std::f32::consts::PI;
    let swing = hand.swing;
    let (sine, root_sine) = ((swing * PI).sin(), (swing.sqrt() * PI).sin());
    let lead = match hand.consume {
        Some((elapsed, duration)) if duration > 0.0 => {
            let remaining = duration - elapsed + 1.0;
            let progress = 1.0 - remaining / duration;
            let bob = if progress > 0.2 {
                (remaining * 0.25 * PI).cos().abs() * 0.1
            } else {
                0.0
            };
            let raise = 1.0 - (1.0 - progress).clamp(0.0, 1.0).powi(27);
            Mat4::from_translation(Vec3::new(0.0, bob, 0.0))
                * Mat4::from_translation(Vec3::new(raise * 0.55, raise * -0.5, 0.0))
                * Mat4::from_rotation_y(degrees(raise * 90.0))
                * Mat4::from_rotation_x(degrees(raise * 10.0))
                * Mat4::from_rotation_z(degrees(raise * 30.0))
        }
        _ => Mat4::from_translation(Vec3::new(
            root_sine * -0.4,
            (swing.sqrt() * PI * 2.0).sin() * 0.2,
            sine * -0.2,
        )),
    };
    let held = lead
        * Mat4::from_translation(Vec3::new(0.56, -0.52, -0.72))
        * Mat4::from_translation(Vec3::new(0.0, (1.0 - hand.equip) * -0.6, 0.0))
        * Mat4::from_rotation_y(degrees(45.0))
        * Mat4::from_rotation_y(degrees((swing * swing * PI).sin() * -20.0))
        * Mat4::from_rotation_z(degrees(root_sine * -20.0))
        * Mat4::from_rotation_x(degrees(root_sine * -80.0))
        * Mat4::from_scale(Vec3::splat(0.4));
    ItemDisplay::from_matrix(match shape {
        FirstPersonShape::Sprite { mirrored_art } => {
            let turn = if mirrored_art {
                Mat4::from_rotation_y(PI)
            } else {
                Mat4::IDENTITY
            };
            held * turn * item_default()
        }
        FirstPersonShape::Block => held,
    })
}

/// A camera-space item bone from `display`; `None` for a non-finite placement.
pub(super) fn view_bone(display: ItemDisplay) -> Option<RenderBoneTransform> {
    let bone = RenderBoneTransform {
        rotation: display.rotation.to_array(),
        translation_scale: [
            display.translation.x,
            display.translation.y,
            display.translation.z,
            display.scale,
        ],
        axis_scale: render::UNIT_AXIS_SCALE,
    };
    bone.is_finite().then_some(bone)
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

/// Items whose icon vanilla turns half a revolution in first person (`isMirroredArt`).
pub(super) fn is_mirrored_art(identifier: &str) -> bool {
    matches!(
        identifier.strip_prefix("minecraft:").unwrap_or(identifier),
        "fishing_rod" | "carrot_on_a_stick" | "warped_fungus_on_a_stick"
    )
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
