//! End portal and end gateway surfaces.
//!
//! The portal is a dark base under additive star layers, each with its own palette color,
//! window size and drift so the layers slide against each other; the gateway is a flat
//! textured plane. Layer count, scales, drift and brightness need native measurement.

use super::{
    atlas::{AtlasRect, BlockEntityAtlas},
    mesh::{Layer, MeshBuilder, WHITE},
    scene::SceneClock,
};

const PORTAL_SURFACE_HEIGHT: f32 = 0.75;
const STAR_LAYERS: usize = 8;
const PALETTE_SIDE: u32 = 4;
const STAR_BRIGHTNESS: f32 = 0.4;
const BASE_COLOR: [f32; 4] = [0.02, 0.05, 0.06, 1.0];
const DRIFT_TICKS: f64 = 600.0;
/// Fraction of the star tile each layer shows, largest window first.
const WINDOWS: [f32; STAR_LAYERS] = [0.9, 0.75, 0.6, 0.5, 0.4, 0.33, 0.25, 0.2];

fn srgb_to_linear(value: u8) -> f32 {
    let value = f32::from(value) / 255.0;
    if value <= 0.04045 {
        value / 12.92
    } else {
        ((value + 0.055) / 1.055).powf(2.4)
    }
}

/// The star window for `layer` at `ticks`: a sub-rect of the tile that drifts back and forth.
#[must_use]
pub fn star_window(rect: AtlasRect, layer: usize, ticks: f64) -> [f32; 4] {
    let window = WINDOWS[layer % STAR_LAYERS];
    let span = 1.0 - window;
    let speed = 1.0 + layer as f64 * 0.37;
    let phase = ticks * speed / DRIFT_TICKS * std::f64::consts::TAU + layer as f64 * 1.7;
    // Different phases per axis keep the drift from tracing a line.
    let (along_x, along_y) = (
        (phase.sin() as f32 * 0.5 + 0.5) * span,
        ((phase * 0.83).cos() as f32 * 0.5 + 0.5) * span,
    );
    [
        rect.x + along_x * rect.width,
        rect.y + along_y * rect.height,
        rect.x + (along_x + window) * rect.width,
        rect.y + (along_y + window) * rect.height,
    ]
}

pub(super) fn emit(
    builder: &mut MeshBuilder,
    atlas: &BlockEntityAtlas,
    block: [i32; 3],
    gateway: bool,
    clock: SceneClock,
) {
    let [bx, by, bz] = block.map(|value| value as f32);
    let y = by + PORTAL_SURFACE_HEIGHT;
    let corners = [
        [bx, y, bz + 1.0],
        [bx + 1.0, y, bz + 1.0],
        [bx + 1.0, y, bz],
        [bx, y, bz],
    ];
    if gateway {
        if let Some(texture) = atlas.texture("textures/blocks/end_gateway", [16.0, 17.0]) {
            builder.textured_quad(Layer::Solid, corners, texture.rect, WHITE);
        }
        return;
    }
    let Some(stars) = atlas.texture("textures/entity/end_portal", [256.0, 256.0]) else {
        return;
    };
    builder.textured_quad(Layer::Solid, corners, stars.rect, BASE_COLOR);
    let palette = "textures/environment/end_portal_colors";
    for layer in 0..STAR_LAYERS {
        let color = atlas
            .texel(
                palette,
                layer as u32 % PALETTE_SIDE,
                layer as u32 / PALETTE_SIDE,
            )
            .map_or([1.0; 3], |[red, green, blue, _]| {
                [red, green, blue].map(srgb_to_linear)
            });
        // Layers rise a hair so additive quads never share depth with the base.
        let lift = 0.0005 * (layer + 1) as f32;
        let raised = corners.map(|[x, height, z]| [x, height + lift, z]);
        builder.quad(
            Layer::Additive,
            raised,
            star_window(stars.rect, layer, clock.ticks),
            [
                color[0] * STAR_BRIGHTNESS,
                color[1] * STAR_BRIGHTNESS,
                color[2] * STAR_BRIGHTNESS,
                1.0,
            ],
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn star_windows_stay_inside_the_tile_and_layers_move_differently() {
        let rect = AtlasRect {
            x: 10.0,
            y: 20.0,
            width: 256.0,
            height: 256.0,
        };
        for layer in 0..STAR_LAYERS {
            for tick in [0.0, 37.0, 411.0, 5_000.0] {
                let [u0, v0, u1, v1] = star_window(rect, layer, tick);
                assert!(u0 >= rect.x - 1.0e-3 && u1 <= rect.x + rect.width + 1.0e-3);
                assert!(v0 >= rect.y - 1.0e-3 && v1 <= rect.y + rect.height + 1.0e-3);
                assert!(u1 > u0 && v1 > v0);
            }
        }
        assert_ne!(star_window(rect, 0, 100.0), star_window(rect, 3, 100.0));
    }
}
