use bevy::prelude::{PerspectiveProjection, Projection, Transform, Vec3};

/// Every actor the renderer can draw is animated: the guard-banded view admits a superset.
#[test]
fn the_animation_view_admits_everything_the_render_cull_draws() {
    let camera =
        Transform::from_xyz(3.0, 70.0, -2.0).looking_at(Vec3::new(20.0, 64.0, 9.0), Vec3::Y);
    let projection = Projection::Perspective(PerspectiveProjection {
        fov: 70f32.to_radians(),
        aspect_ratio: 16.0 / 9.0,
        ..Default::default()
    });
    let view = super::animation_view(&camera, &projection).unwrap();
    let cull = render::ActorCullView {
        clip_from_world: projection.get_clip_from_view() * camera.to_matrix().inverse(),
        camera_position: camera.translation,
        max_distance: render::MAX_ACTOR_RENDER_DISTANCE_BLOCKS,
    };
    let (mut drawn, mut held) = (0, 0);
    for x in (-60..=60).step_by(3) {
        for z in (-60..=60).step_by(3) {
            for (y, scale) in [(60.0, 1.0), (64.0, 0.01), (75.0, 3.0)] {
                let feet = [x as f32, y, z as f32];
                if render::actor_bounds_are_visible(feet, scale, Default::default(), Some(cull)) {
                    drawn += 1;
                    assert!(
                        view.admits(feet, scale, false, Default::default()),
                        "{feet:?} x{scale}"
                    );
                } else if !view.admits(feet, scale, false, Default::default()) {
                    held += 1;
                }
            }
        }
    }
    assert!(drawn > 100 && held > 100, "drawn={drawn} held={held}");
}

use client_world::HandPhase;

// The swing wraps forward from its last tick to rest, and an eat use counts from its first
// using tick only while the rig reports the use.
#[test]
fn hand_progress_interpolates_the_swing_forward_and_counts_the_use() {
    let phase = |attack_time, arm_height, use_ticks| HandPhase {
        attack_time,
        arm_height,
        use_ticks,
    };
    let hand = super::hand_progress([phase(5.0 / 6.0, 0.6, 0), phase(0.0, 1.0, 0)], None, 0.5);
    assert!((hand.swing - 11.0 / 12.0).abs() < 1e-6);
    assert!((hand.equip - 0.8).abs() < 1e-6);
    assert_eq!(hand.consume, None);
    let eating = super::hand_progress([phase(0.0, 1.0, 3), phase(0.0, 1.0, 4)], Some(32), 0.25);
    assert_eq!(eating.consume, Some((3.25, 32.0)));
    let idle = super::hand_progress([phase(0.0, 1.0, 0), phase(0.0, 1.0, 0)], Some(32), 0.25);
    assert_eq!(idle.consume, None);
}

// The pack's first-person arm offset sits behind the model's left side; vanilla's facing puts
// that ahead of the view and to its right.
#[test]
fn first_person_arm_offset_lands_ahead_and_right_of_the_camera() {
    let rows = super::hand_camera_from_rig(0.9375, bevy::math::Mat4::IDENTITY);
    let arm = [-8.5 / 16.0, 12.0 / 16.0, 12.0 / 16.0];
    let camera: [f32; 3] = std::array::from_fn(|row| {
        (0..3).map(|axis| rows[row][axis] * arm[axis]).sum::<f32>() + rows[row][3]
    });
    assert!(
        camera[0] > 0.0 && camera[1] < 0.0 && camera[2] < 0.0,
        "{camera:?}"
    );
    assert!(
        (rows[1][3] + crate::local_player::LOCAL_AVATAR_EYE_HEIGHT_BLOCKS - 0.9375 / 128.0).abs()
            < 1e-6
    );
}
