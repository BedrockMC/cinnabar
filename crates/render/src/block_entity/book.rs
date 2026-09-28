//! The open book on enchanting tables and lecterns.
//!
//! Part boxes and UV origins follow the enchanting-book texture unwrap; the hover height,
//! tilt, spread and page-flip animation need native measurement, and pages do not flip yet.

use bevy::math::Mat4;

use super::{
    atlas::BlockEntityAtlas,
    mesh::{BoxSpec, Layer, MeshBuilder, WHITE, model_matrix},
    scene::SceneClock,
};

const HOVER_HEIGHT_PIXELS: f32 = 12.0;
const BOB_PIXELS: f32 = 0.6;
const BOB_PERIOD_TICKS: f64 = 63.0;
const SPREAD_DEGREES: f32 = 20.0;
const TILT_DEGREES: f32 = 10.0;
const THIN: f32 = 0.01;

/// Emits the book with its spine along local Y, pages facing +Z, in `matrix` space.
fn emit_book(builder: &mut MeshBuilder, atlas: &BlockEntityAtlas, matrix: Mat4) {
    let Some(texture) = atlas.texture("textures/entity/enchanting_table_book", [64.0, 32.0]) else {
        return;
    };
    let spread = SPREAD_DEGREES.to_radians();
    let left = matrix * Mat4::from_rotation_y(spread);
    let right = matrix * Mat4::from_rotation_y(-spread);
    let parts: [(Mat4, BoxSpec); 5] = [
        (
            left,
            BoxSpec::new([-6.0, -5.0, -THIN], [6.0, 10.0, THIN], [0.0, 0.0]),
        ),
        (
            right,
            BoxSpec::new([0.0, -5.0, -THIN], [6.0, 10.0, THIN], [16.0, 0.0]),
        ),
        (
            matrix,
            BoxSpec::new([-1.0, -5.0, -THIN], [2.0, 10.0, THIN], [12.0, 0.0]),
        ),
        (
            left,
            BoxSpec::new([-5.0, -4.0, 0.0], [5.0, 8.0, 1.0], [0.0, 10.0]),
        ),
        (
            right,
            BoxSpec::new([0.0, -4.0, 0.0], [5.0, 8.0, 1.0], [12.0, 10.0]),
        ),
    ];
    for (part_matrix, spec) in parts {
        builder.cuboid(Layer::Solid, &texture, part_matrix, spec, WHITE);
    }
    builder.cuboid(
        Layer::Solid,
        &texture,
        matrix,
        BoxSpec::new([0.0, -4.0, 0.5], [5.0, 8.0, THIN], [24.0, 10.0]),
        WHITE,
    );
}

pub(super) fn emit_enchant_table(
    builder: &mut MeshBuilder,
    atlas: &BlockEntityAtlas,
    block: [i32; 3],
    facing_yaw_degrees: f32,
    clock: SceneClock,
) {
    let phase = (clock.ticks / BOB_PERIOD_TICKS).fract() as f32;
    let bob = BOB_PIXELS * (std::f32::consts::TAU * phase).sin();
    // Lay the book flat (pages up) and tip it toward the viewer.
    let matrix = model_matrix(
        block,
        [0.5, (HOVER_HEIGHT_PIXELS + bob) / 16.0, 0.5],
        facing_yaw_degrees,
    ) * Mat4::from_rotation_x(-TILT_DEGREES.to_radians())
        * Mat4::from_rotation_x(-std::f32::consts::FRAC_PI_2);
    emit_book(builder, atlas, matrix);
}

pub(super) fn emit_lectern(
    builder: &mut MeshBuilder,
    atlas: &BlockEntityAtlas,
    block: [i32; 3],
    facing_yaw_degrees: f32,
) {
    // The lectern top slopes about 22.5 degrees toward the reader.
    let matrix = model_matrix(block, [0.5, 1.0, 0.5], facing_yaw_degrees)
        * Mat4::from_rotation_x(-(22.5_f32).to_radians())
        * Mat4::from_rotation_x(-std::f32::consts::FRAC_PI_2);
    emit_book(builder, atlas, matrix);
}

#[cfg(test)]
mod tests {
    use bevy::math::Vec3;

    use super::*;

    #[test]
    fn flattening_turns_the_page_normal_upward() {
        let normal = Mat4::from_rotation_x(-std::f32::consts::FRAC_PI_2).transform_vector3(Vec3::Z);
        assert!(normal.abs_diff_eq(Vec3::Y, 1.0e-5));
    }
}
