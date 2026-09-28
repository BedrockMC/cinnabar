//! End portal and end gateway surfaces.
//!
//! Provisional: a flat textured plane at the vanilla surface height with a slow drift; the
//! layered parallax starfield needs native measurement.

use super::{
    atlas::BlockEntityAtlas,
    mesh::{Layer, MeshBuilder, WHITE},
    scene::SceneClock,
};

const PORTAL_SURFACE_HEIGHT: f32 = 0.75;
const DRIFT_TICKS_PER_LOOP: f64 = 400.0;

pub(super) fn emit(
    builder: &mut MeshBuilder,
    atlas: &BlockEntityAtlas,
    block: [i32; 3],
    gateway: bool,
    clock: SceneClock,
) {
    let (name, logical) = if gateway {
        ("textures/blocks/end_gateway", [16.0, 17.0])
    } else {
        ("textures/entity/end_portal", [256.0, 256.0])
    };
    let Some(texture) = atlas.texture(name, logical) else {
        return;
    };
    let [bx, by, bz] = block.map(|value| value as f32);
    let y = by + PORTAL_SURFACE_HEIGHT;
    let drift = (clock.ticks / DRIFT_TICKS_PER_LOOP).fract() as f32 * texture.rect.width;
    let rect = texture.rect;
    let corners = [
        [bx, y, bz + 1.0],
        [bx + 1.0, y, bz + 1.0],
        [bx + 1.0, y, bz],
        [bx, y, bz],
    ];
    // The drift shifts the sampled window within the tile; the tile edge clamps.
    let shift = drift.min(rect.width * 0.25);
    builder.quad(
        Layer::Solid,
        corners,
        [
            rect.x + shift,
            rect.y + shift,
            rect.x + rect.width * 0.75 + shift,
            rect.y + rect.height * 0.75 + shift,
        ],
        WHITE,
    );
}
