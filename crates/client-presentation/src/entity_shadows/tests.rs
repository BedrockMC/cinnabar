use super::*;
use bevy::math::Mat4;

fn view() -> ActorCullView {
    let camera = Vec3::new(0.0, 70.0, 0.0);
    let clip_from_view = Mat4::perspective_infinite_reverse_rh(1.2, 1.0, 0.05);
    let view_from_world = Mat4::look_to_rh(camera, Vec3::NEG_Z, Vec3::Y);
    ActorCullView {
        clip_from_world: clip_from_view * view_from_world,
        camera_position: camera,
        max_distance: render::MAX_ACTOR_RENDER_DISTANCE_BLOCKS,
    }
}

fn at(x: f32, y: f32, z: f32) -> EntityShadow {
    EntityShadow {
        feet: [x, y, z],
        radius: 0.6,
    }
}

#[test]
fn casters_ahead_are_kept_and_casters_behind_are_culled() {
    assert!(volume_may_be_visible(&at(0.0, 66.0, -8.0), false, view()));
    assert!(!volume_may_be_visible(&at(0.0, 66.0, 8.0), false, view()));
}

/// A shadow whose volume reaches into view is kept even when its feet are just off screen.
#[test]
fn a_volume_reaching_into_view_is_kept() {
    // The view spans y 68.6..71.4 two blocks ahead; the volume hangs 1.8 below the feet.
    assert!(volume_may_be_visible(&at(0.0, 72.0, -2.0), false, view()));
    assert!(!volume_may_be_visible(&at(0.0, 74.0, -2.0), false, view()));
}

#[test]
fn entities_outside_the_candidate_cube_are_culled_but_players_are_not() {
    let far = at(0.0, 64.0, -(render::ACTOR_CANDIDATE_RADIUS_BLOCKS + 4.0));
    assert!(!volume_may_be_visible(&far, false, view()));
    assert_eq!(
        volume_may_be_visible(&far, true, view()),
        render::ACTOR_CANDIDATE_RADIUS_BLOCKS + 4.0 <= render::MAX_ACTOR_RENDER_DISTANCE_BLOCKS
    );
}
