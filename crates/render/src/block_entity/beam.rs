//! Beacon beam: two nested translucent columns rising from the beacon.
//!
//! Radii and tint blending need native measurement; texture scroll and the glass-tinted
//! beam color are not implemented yet.

use bevy::math::Vec3;

use super::{
    atlas::BlockEntityAtlas,
    mesh::{Layer, MeshBuilder},
    scene::SceneClock,
};

const INNER_RADIUS: f32 = 0.2;
const OUTER_RADIUS: f32 = 0.25;
const INNER_ALPHA: f32 = 0.8;
const OUTER_ALPHA: f32 = 0.3;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BeaconModel {
    /// Beam height in blocks above the beacon block; zero draws nothing.
    pub height: u32,
    /// Linear RGB tint.
    pub tint: [f32; 3],
}

pub(super) fn emit(
    builder: &mut MeshBuilder,
    atlas: &BlockEntityAtlas,
    block: [i32; 3],
    model: &BeaconModel,
    _clock: SceneClock,
) {
    if model.height == 0 {
        return;
    }
    let Some(texture) = atlas.texture("textures/entity/beacon_beam", [16.0, 16.0]) else {
        return;
    };
    let center = Vec3::new(
        block[0] as f32 + 0.5,
        block[1] as f32 + 1.0,
        block[2] as f32 + 0.5,
    );
    let height = model.height as f32;
    for (radius, alpha) in [(INNER_RADIUS, INNER_ALPHA), (OUTER_RADIUS, OUTER_ALPHA)] {
        let color = [model.tint[0], model.tint[1], model.tint[2], alpha];
        let (x0, x1) = (center.x - radius, center.x + radius);
        let (z0, z1) = (center.z - radius, center.z + radius);
        let (y0, y1) = (center.y, center.y + height);
        let sides = [
            [[x1, y1, z0], [x0, y1, z0], [x0, y0, z0], [x1, y0, z0]],
            [[x0, y1, z1], [x1, y1, z1], [x1, y0, z1], [x0, y0, z1]],
            [[x1, y1, z1], [x1, y1, z0], [x1, y0, z0], [x1, y0, z1]],
            [[x0, y1, z0], [x0, y1, z1], [x0, y0, z1], [x0, y0, z0]],
        ];
        for corners in sides {
            // One texture tile per block of height; the atlas rect cannot wrap, so tall
            // beams repeat the tile per block.
            for step in 0..model.height {
                let lift = step as f32;
                let quad = corners.map(|corner| {
                    let mut corner = corner;
                    corner[1] = corner[1].min(y0 + lift + 1.0).max(y0 + lift);
                    corner
                });
                builder.textured_quad(Layer::Overlay, quad, texture.rect, color);
            }
        }
    }
}
