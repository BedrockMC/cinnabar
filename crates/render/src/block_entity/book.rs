//! The open book on enchanting tables and lecterns, and the lectern stand.
//!
//! Book part boxes and UV origins follow the enchanting-book texture unwrap; the hover height,
//! tilt, spread, page-flip timing and the lectern stand's dimensions need native measurement.

use bevy::math::Mat4;

use super::{
    atlas::{AtlasRect, BlockEntityAtlas},
    mesh::{BoxSpec, Layer, MeshBuilder, WHITE, model_matrix},
    scene::SceneClock,
};

const HOVER_HEIGHT_PIXELS: f32 = 12.0;
const BOB_PIXELS: f32 = 0.6;
const BOB_PERIOD_TICKS: f64 = 63.0;
const SPREAD_DEGREES: f32 = 20.0;
const SPREAD_WOBBLE_DEGREES: f32 = 4.0;
const TILT_DEGREES: f32 = 10.0;
const FLIP_PERIOD_TICKS: f64 = 40.0;
const THIN: f32 = 0.01;
const LECTERN_SLOPE_DEGREES: f32 = 22.5;
const LECTERN_BOARD_HEIGHT: f32 = 14.0;

/// Emits the book with its spine along local Y, pages facing +Z, in `matrix` space; `flip`
/// is the turning page's angle about the spine in radians.
fn emit_book(
    builder: &mut MeshBuilder,
    atlas: &BlockEntityAtlas,
    matrix: Mat4,
    spread: f32,
    flip: f32,
) {
    let Some(texture) = atlas.texture("textures/entity/enchanting_table_book", [64.0, 32.0]) else {
        return;
    };
    let left = matrix * Mat4::from_rotation_y(spread);
    let right = matrix * Mat4::from_rotation_y(-spread);
    let turning = matrix * Mat4::from_rotation_y(flip);
    let parts: [(Mat4, BoxSpec); 6] = [
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
        (
            turning,
            BoxSpec::new([0.0, -4.0, 0.5], [5.0, 8.0, THIN], [24.0, 10.0]),
        ),
    ];
    for (part_matrix, spec) in parts {
        builder.cuboid(Layer::Solid, &texture, part_matrix, spec, WHITE);
    }
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
    let flip_phase = (clock.ticks / FLIP_PERIOD_TICKS).fract() as f32;
    let spread = SPREAD_DEGREES.to_radians()
        + SPREAD_WOBBLE_DEGREES.to_radians() * (std::f32::consts::TAU * phase).sin();
    // The turning page sweeps between the two covers.
    let flip = spread * (std::f32::consts::TAU * flip_phase).cos();
    // Lay the book flat (pages up) and tip it toward the viewer.
    let matrix = model_matrix(
        block,
        [0.5, (HOVER_HEIGHT_PIXELS + bob) / 16.0, 0.5],
        facing_yaw_degrees,
    ) * Mat4::from_rotation_x(-TILT_DEGREES.to_radians())
        * Mat4::from_rotation_x(-std::f32::consts::FRAC_PI_2);
    emit_book(builder, atlas, matrix, spread, flip);
}

pub(super) fn emit_lectern(
    builder: &mut MeshBuilder,
    atlas: &BlockEntityAtlas,
    block: [i32; 3],
    facing_yaw_degrees: f32,
    has_book: bool,
) {
    let base = model_matrix(block, [0.5, 0.0, 0.5], facing_yaw_degrees);
    let tile = |name: &str| {
        atlas
            .texture(name, [16.0, 16.0])
            .map(|texture| texture.rect)
    };
    if let (Some(floor), Some(sides), Some(top), Some(front)) = (
        tile("textures/blocks/lectern_base"),
        tile("textures/blocks/lectern_sides"),
        tile("textures/blocks/lectern_top"),
        tile("textures/blocks/lectern_front"),
    ) {
        builder.tile_cuboid(
            Layer::Solid,
            base,
            [-8.0, 0.0, -8.0],
            [8.0, 2.0, 8.0],
            [floor; 6],
            WHITE,
        );
        builder.tile_cuboid(
            Layer::Solid,
            base,
            [-4.0, 2.0, -4.0],
            [4.0, 14.0, 4.0],
            [sides; 6],
            WHITE,
        );
        // The reading board slopes down toward the reader (-Z).
        let board =
            base * bevy::math::Mat4::from_translation(bevy::math::Vec3::new(
                0.0,
                LECTERN_BOARD_HEIGHT,
                0.0,
            )) * Mat4::from_rotation_x(-LECTERN_SLOPE_DEGREES.to_radians());
        let rects: [AtlasRect; 6] = [sides, sides, sides, top, front, sides];
        builder.tile_cuboid(
            Layer::Solid,
            board,
            [-8.0, 0.0, -8.0],
            [8.0, 2.0, 8.0],
            rects,
            WHITE,
        );
        if has_book {
            let book = board
                * Mat4::from_translation(bevy::math::Vec3::new(0.0, 2.5, 0.0))
                * Mat4::from_rotation_x(-std::f32::consts::FRAC_PI_2);
            emit_book(builder, atlas, book, SPREAD_DEGREES.to_radians(), 0.0);
        }
    }
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

    #[test]
    fn the_lectern_slope_lowers_the_reader_edge() {
        let slope = Mat4::from_rotation_x(-LECTERN_SLOPE_DEGREES.to_radians());
        let reader_edge = slope.transform_point3(Vec3::new(0.0, 0.0, -8.0));
        let far_edge = slope.transform_point3(Vec3::new(0.0, 0.0, 8.0));
        assert!(reader_edge.y < far_edge.y);
    }
}
