use bevy::math::{Mat4, Vec2, Vec3, Vec4};

/// Bounds the visible portion of an entity box in physical pixels, clipping its near plane.
pub fn project_entity_rectangle(
    eye: Vec3,
    clip_from_world: Mat4,
    bounds: [[f32; 3]; 2],
    viewport: [u32; 2],
) -> Option<[u32; 4]> {
    let [min, max] = bounds.map(Vec3::from_array);
    if viewport.contains(&0)
        || !eye.is_finite()
        || !clip_from_world.is_finite()
        || !min.is_finite()
        || !max.is_finite()
        || !min.cmplt(max).all()
        || (eye.cmpge(min).all() && eye.cmple(max).all())
    {
        return None;
    }
    let corners: [Vec4; 8] = std::array::from_fn(|index| {
        clip_from_world
            * Vec3::new(
                if index & 1 == 0 { min.x } else { max.x },
                if index & 2 == 0 { min.y } else { max.y },
                if index & 4 == 0 { min.z } else { max.z },
            )
            .extend(1.0)
    });
    let mut lower = Vec2::splat(f32::INFINITY);
    let mut upper = Vec2::splat(f32::NEG_INFINITY);
    let mut admit = |point: Vec4| {
        if point.is_finite() && point.w > f32::EPSILON && point.z >= 0.0 {
            let ndc = point.truncate() / point.w;
            let xy = Vec2::new(ndc.x, -ndc.y);
            lower = lower.min(xy);
            upper = upper.max(xy);
        }
    };
    for point in corners {
        if point.w - point.z >= 0.0 {
            admit(point);
        }
    }
    for index in 0..8 {
        for axis in 0..3 {
            let other = index ^ (1 << axis);
            if index >= other {
                continue;
            }
            let (a, b) = (corners[index], corners[other]);
            let (da, db) = (a.w - a.z, b.w - b.z);
            if (da >= 0.0) != (db >= 0.0) {
                admit(a.lerp(b, da / (da - db)));
            }
        }
    }
    if !lower.is_finite()
        || !upper.is_finite()
        || lower.x >= 1.0
        || lower.y >= 1.0
        || upper.x <= -1.0
        || upper.y <= -1.0
    {
        return None;
    }
    let size = Vec2::new(viewport[0] as f32, viewport[1] as f32);
    let lower = ((lower.max(Vec2::NEG_ONE) + Vec2::ONE) * 0.5 * size).floor();
    let upper = ((upper.min(Vec2::ONE) + Vec2::ONE) * 0.5 * size).ceil();
    let rect = [
        lower.x as u32,
        lower.y as u32,
        upper.x as u32,
        upper.y as u32,
    ];
    (rect[2] > rect[0] + 1 && rect[3] > rect[1] + 1).then_some(rect)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn projection() -> Mat4 {
        Mat4::perspective_infinite_reverse_rh(std::f32::consts::FRAC_PI_2, 1.0, 0.1)
    }

    #[test]
    fn front_box_has_expected_perspective_extent_and_scales_with_viewport() {
        let bounds = [[-0.5, -0.5, -3.0], [0.5, 0.5, -2.0]];
        assert_eq!(
            project_entity_rectangle(Vec3::ZERO, projection(), bounds, [400, 400]),
            Some([150, 150, 250, 250])
        );
        assert_eq!(
            project_entity_rectangle(Vec3::ZERO, projection(), bounds, [800, 800]),
            Some([300, 300, 500, 500])
        );
    }

    #[test]
    fn near_plane_crossing_uses_clipped_edges_instead_of_dividing_behind_camera() {
        let bounds = [[0.02, -0.01, -0.2], [0.03, 0.01, 0.2]];
        let rect = project_entity_rectangle(Vec3::ZERO, projection(), bounds, [400, 400]).unwrap();
        assert_eq!(rect, [220, 180, 260, 220]);
    }

    #[test]
    fn behind_offscreen_inside_and_invalid_boxes_have_no_rectangle() {
        for bounds in [
            [[-0.5, -0.5, 2.0], [0.5, 0.5, 3.0]],
            [[3.0, -0.5, -2.0], [4.0, 0.5, -1.0]],
            [[-0.5, -0.5, -0.5], [0.5, 0.5, 0.5]],
            [[f32::NAN, -0.5, -3.0], [0.5, 0.5, -2.0]],
        ] {
            assert_eq!(
                project_entity_rectangle(Vec3::ZERO, projection(), bounds, [400, 400]),
                None
            );
        }
    }

    #[test]
    fn partial_screen_bounds_are_clamped_to_the_viewport() {
        let rect = project_entity_rectangle(
            Vec3::ZERO,
            projection(),
            [[-2.0, -0.5, -2.0], [-0.5, 0.5, -1.0]],
            [400, 400],
        )
        .unwrap();
        assert_eq!(rect, [0, 100, 150, 300]);
    }
}
