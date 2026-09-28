//! Item and glow item frames: a flat backing panel on the wall; the framed item is drawn by
//! the dropped-item renderer at the pose from [`item_frame_item_transform`].
//!
//! Panel size and item scale are provisional and need native measurement.

use bevy::math::{Mat4, Vec3};

use super::{
    atlas::BlockEntityAtlas,
    items::matrix_rows,
    mesh::{Facing, Layer, MeshBuilder, WHITE},
};

const PANEL_HALF: f32 = 6.0;
/// Panel depth in pixels, flush with the wall behind it.
const PANEL_FRONT: f32 = 7.0;
const PANEL_BACK: f32 = 8.0;
/// Local depth of the item plane, in front of the panel.
const ITEM_DEPTH: f32 = 6.0;
const ITEM_SCALE: f32 = 0.5;
const ROTATION_STEP_DEGREES: f32 = 45.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ItemFrameModel {
    pub glow: bool,
    /// Direction the frame faces, as a Bedrock face id: 0 down, 1 up, 2 north, 3 south, 4 west,
    /// 5 east; other values read as north.
    pub outward: u8,
}

/// Maps frame-local block-center space (front toward -Z, pixels scaled to blocks) into the world.
fn frame_matrix(block: [i32; 3], outward: u8) -> Mat4 {
    let center = Vec3::new(
        block[0] as f32 + 0.5,
        block[1] as f32 + 0.5,
        block[2] as f32 + 0.5,
    );
    let turn = match outward {
        0 => Mat4::from_rotation_x(-std::f32::consts::FRAC_PI_2),
        1 => Mat4::from_rotation_x(std::f32::consts::FRAC_PI_2),
        other => Mat4::from_rotation_y(
            Facing::from_facing_direction(i64::from(other))
                .unwrap_or(Facing::North)
                .yaw_degrees()
                .to_radians(),
        ),
    };
    Mat4::from_translation(center) * turn
}

pub(super) fn emit(
    builder: &mut MeshBuilder,
    atlas: &BlockEntityAtlas,
    block: [i32; 3],
    model: &ItemFrameModel,
) {
    let name = if model.glow {
        "textures/blocks/glow_item_frame"
    } else {
        "textures/blocks/itemframe_background"
    };
    let Some(texture) = atlas.texture(name, [16.0, 16.0]) else {
        return;
    };
    let matrix = frame_matrix(block, model.outward) * Mat4::from_scale(Vec3::splat(1.0 / 16.0));
    builder.tile_cuboid(
        Layer::Solid,
        matrix,
        [-PANEL_HALF, -PANEL_HALF, PANEL_FRONT],
        [PANEL_HALF, PANEL_HALF, PANEL_BACK],
        [texture.rect; 6],
        WHITE,
    );
}

/// Pose of the framed item; `rotation_steps` counts 45-degree turns.
#[must_use]
pub fn item_frame_item_transform(
    block: [i32; 3],
    outward: u8,
    rotation_steps: u8,
) -> [[f32; 4]; 3] {
    // The sprite faces +Z, so a half turn points it at the viewer in front of the panel.
    let matrix = frame_matrix(block, outward)
        * Mat4::from_translation(Vec3::new(0.0, 0.0, ITEM_DEPTH / 16.0))
        * Mat4::from_rotation_y(std::f32::consts::PI)
        * Mat4::from_rotation_z(-(f32::from(rotation_steps) * ROTATION_STEP_DEGREES).to_radians())
        * Mat4::from_scale(Vec3::splat(ITEM_SCALE));
    matrix_rows(matrix)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_item_sits_in_front_of_the_wall_panel() {
        // North-facing frame at the origin: the item plane is toward -Z of the block center
        // relative to the panel, so it is nearer the north edge than the wall (south) edge.
        let rows = item_frame_item_transform([0, 0, 0], 2, 0);
        let z = rows[2][3];
        assert!(z > 0.5 && z < 1.0, "{z}");
        assert!((rows[0][3] - 0.5).abs() < 1.0e-6 && (rows[1][3] - 0.5).abs() < 1.0e-6);
    }

    #[test]
    fn outward_ids_pick_the_turn() {
        let up = frame_matrix([0; 3], 1).transform_vector3(Vec3::NEG_Z);
        let down = frame_matrix([0; 3], 0).transform_vector3(Vec3::NEG_Z);
        let west = frame_matrix([0; 3], 4).transform_vector3(Vec3::NEG_Z);
        assert!(up.abs_diff_eq(Vec3::Y, 1.0e-5));
        assert!(down.abs_diff_eq(Vec3::NEG_Y, 1.0e-5));
        assert!(west.abs_diff_eq(Vec3::NEG_X, 1.0e-5));
    }
}
