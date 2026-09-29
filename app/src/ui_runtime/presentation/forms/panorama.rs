//! The launcher's panorama backdrop: the carrier's six cube faces seen from the
//! cube's centre, turning slowly. Side faces are drawn as texel-column strips
//! whose height follows their perspective depth, so the view is rectilinear
//! without a 3D pass; the top and bottom faces fill behind them.

use std::{
    f32::consts::{FRAC_PI_2, PI},
    sync::OnceLock,
    time::Instant,
};

use ui::{UiNode, UiNodeId, UiVisual};

use super::super::{IconRef, UiPresentationError, rect};
use super::engine::FormEngine;

/// Vertical field of view; needs native measurement.
const VERTICAL_FOV: f32 = 85.0 * PI / 180.0;
/// Seconds per full turn; needs native measurement.
const TURN_SECONDS: f32 = 240.0;
/// Face texels per strip.
const STRIP_TEXELS: u16 = 4;

/// Draws the panorama over the whole content area; `Ok(false)` when the
/// carrier lacks the faces.
pub(super) fn append_panorama(
    engine: &FormEngine,
    nodes: &mut Vec<UiNode>,
    next: &mut u32,
    size: [f32; 2],
) -> Result<bool, UiPresentationError> {
    let faces: Option<Vec<IconRef>> = (0..6)
        .map(|face| engine.atlas_sprite(&format!("textures/ui/panorama_{face}")))
        .collect();
    let Some(faces) = faces else {
        return Ok(false);
    };
    let [width, height] = size;
    if width <= 0.0 || height <= 0.0 {
        return Ok(true);
    }
    let mut push = |bounds: [f32; 4], icon: IconRef, uv: [u16; 4]| {
        let node = UiNode::new(
            UiNodeId::new(*next),
            None,
            rect(bounds[0], bounds[1], bounds[2], bounds[3])?,
        )
        .with_visual(UiVisual::Sprite {
            texture_page: icon.page,
            uv,
            color: [255; 4],
        });
        nodes.push(node);
        *next = next.saturating_add(1);
        Ok::<(), UiPresentationError>(())
    };
    // Top and bottom only show past the side faces' edges.
    push([0.0, 0.0, width, height * 0.5], faces[4], faces[4].uv)?;
    push([0.0, height * 0.5, width, height], faces[5], faces[5].uv)?;
    let half = height * 0.5;
    let focal = 1.0 / (VERTICAL_FOV * 0.5).tan();
    static EPOCH: OnceLock<Instant> = OnceLock::new();
    let seconds = EPOCH.get_or_init(Instant::now).elapsed().as_secs_f32();
    let yaw = (seconds / TURN_SECONDS).fract() * 2.0 * PI;
    for (index, face) in faces.iter().take(4).enumerate() {
        let side = face.uv[2].saturating_sub(face.uv[0]);
        let centre = index as f32 * FRAC_PI_2;
        let local = |t: u16| (2.0 * f32::from(t) / f32::from(side.max(1)) - 1.0).atan();
        let mut start = 0;
        while start < side {
            let end = start.saturating_add(STRIP_TEXELS).min(side);
            let (from, to) = (local(start), local(end));
            let uv = [face.uv[0] + start, face.uv[1], face.uv[0] + end, face.uv[3]];
            start = end;
            let relative = |angle: f32| wrap(centre + angle - yaw);
            let (left, right) = (relative(from), relative(to));
            // Behind the camera, or wrapping across it.
            if left.abs() >= 1.5 || right.abs() >= 1.5 || right <= left {
                continue;
            }
            let x = |angle: f32| width * 0.5 + focal * angle.tan() * half;
            let (x0, x1) = (x(left), x(right));
            if x1 <= 0.0 || x0 >= width {
                continue;
            }
            let depth = relative((from + to) * 0.5).cos() / ((from + to) * 0.5).cos();
            let extent = focal / depth * half;
            push([x0, half - extent, x1, half + extent], *face, uv)?;
        }
    }
    Ok(true)
}

/// An angle wrapped into `(-π, π]`.
fn wrap(angle: f32) -> f32 {
    let wrapped = (angle + PI).rem_euclid(2.0 * PI) - PI;
    if wrapped <= -PI {
        wrapped + 2.0 * PI
    } else {
        wrapped
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn angles_wrap_into_one_turn_around_zero() {
        assert!((wrap(3.0 * PI / 2.0) + FRAC_PI_2).abs() < 1e-5);
        assert!((wrap(-3.0 * PI / 2.0) - FRAC_PI_2).abs() < 1e-5);
        assert!((wrap(0.25) - 0.25).abs() < 1e-6);
    }
}
