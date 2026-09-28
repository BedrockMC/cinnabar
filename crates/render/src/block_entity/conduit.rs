//! Conduits: a shell cube, plus a spinning cage and a viewer-facing eye while active.
//!
//! Box sizes follow the conduit textures (6x6x6 shell, 8x8x8 cage); the animated wind cube is
//! not drawn, and bob, spin and eye size need native measurement.

use bevy::math::{Mat4, Vec3};

use super::{
    atlas::BlockEntityAtlas,
    mesh::{BoxSpec, Layer, MeshBuilder, WHITE, model_matrix},
    scene::SceneClock,
};

const SPIN_DEGREES_PER_TICK: f64 = 2.0;
const BOB_PIXELS: f32 = 1.6;
const BOB_PERIOD_TICKS: f64 = 40.0;
const EYE_HALF_PIXELS: f32 = 2.0;
const ACTIVE_HEIGHT_PIXELS: f32 = 8.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ConduitModel {
    pub active: bool,
    /// Whether a hostile target is being attacked; opens the eye.
    pub hunting: bool,
    /// Yaw that turns the eye quad toward the viewer.
    pub viewer_yaw_degrees: f32,
}

pub(super) fn emit(
    builder: &mut MeshBuilder,
    atlas: &BlockEntityAtlas,
    block: [i32; 3],
    model: &ConduitModel,
    clock: SceneClock,
) {
    let phase = (clock.ticks / BOB_PERIOD_TICKS).fract() as f32;
    let bob = if model.active {
        BOB_PIXELS * (std::f32::consts::TAU * phase).sin()
    } else {
        0.0
    };
    let center = model_matrix(block, [0.5, 0.0, 0.5], 0.0)
        * Mat4::from_translation(Vec3::new(0.0, ACTIVE_HEIGHT_PIXELS + bob, 0.0));
    if let Some(shell) = atlas.texture("textures/blocks/conduit_base", [24.0, 12.0]) {
        builder.cuboid(
            Layer::Solid,
            &shell,
            center,
            BoxSpec::new([-3.0, -3.0, -3.0], [6.0, 6.0, 6.0], [0.0, 0.0]),
            WHITE,
        );
    }
    if !model.active {
        return;
    }
    if let Some(cage) = atlas.texture("textures/blocks/conduit_cage", [32.0, 16.0]) {
        let spin = Mat4::from_rotation_y(
            ((clock.ticks * SPIN_DEGREES_PER_TICK) % 360.0).to_radians() as f32,
        );
        builder.cuboid(
            Layer::Solid,
            &cage,
            center * spin,
            BoxSpec::new([-4.0, -4.0, -4.0], [8.0, 8.0, 8.0], [0.0, 0.0]),
            WHITE,
        );
    }
    let eye = if model.hunting {
        "textures/blocks/conduit_open"
    } else {
        "textures/blocks/conduit_closed"
    };
    if let Some(eye) = atlas.texture(eye, [8.0, 8.0]) {
        let facing = center * Mat4::from_rotation_y(model.viewer_yaw_degrees.to_radians());
        let corners = [
            [EYE_HALF_PIXELS, EYE_HALF_PIXELS, -3.5],
            [-EYE_HALF_PIXELS, EYE_HALF_PIXELS, -3.5],
            [-EYE_HALF_PIXELS, -EYE_HALF_PIXELS, -3.5],
            [EYE_HALF_PIXELS, -EYE_HALF_PIXELS, -3.5],
        ]
        .map(|corner| facing.transform_point3(Vec3::from_array(corner)).to_array());
        builder.textured_quad(Layer::Solid, corners, eye.rect, WHITE);
    }
}
