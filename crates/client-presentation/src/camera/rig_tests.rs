use bevy::prelude::{EulerRot, Quat, Vec3};
use semantic_input::PerspectiveMode;
use sim::{
    Aabb, CollisionQuery, CollisionWorld, LenientSkipCounts, Vec3 as SimVec3, WorldQueryError,
};

use super::{
    CameraRig, CameraSettingsAuthority, THIRD_PERSON_COLLISION_EPSILON_BLOCKS,
    collision_safe_rig_pose, rig_pose,
};

struct Walls(Vec<Aabb>);

impl CollisionWorld for Walls {
    /// Visits retained fixture shapes without constructing an owned query result.
    fn visit_collision_boxes_camera_lenient(
        &self,
        _query: Aabb,
        visitor: &mut dyn FnMut(Aabb),
    ) -> Result<LenientSkipCounts, WorldQueryError> {
        for shape in &self.0 {
            visitor(*shape);
        }
        Ok(LenientSkipCounts::default())
    }

    fn collision_boxes(&self, _query: Aabb) -> Result<CollisionQuery<Vec<Aabb>>, WorldQueryError> {
        Ok(CollisionQuery::synthetic(self.0.clone()))
    }
}

fn shoulder() -> CameraRig {
    CameraRig {
        offset: Vec3::new(0.8, 0.4, 3.0),
        roll_radians: 0.0,
        fov_delta_degrees: 0.0,
    }
}

#[test]
fn rig_forces_third_person_back_and_clearing_restores_the_player_choice() {
    let mut settings = CameraSettingsAuthority::default();
    settings.reset_perspective();
    settings.set_rig(Some(shoulder()));
    assert_eq!(settings.perspective(), PerspectiveMode::ThirdPersonBack);
    settings.set_rig(None);
    assert_eq!(settings.perspective(), PerspectiveMode::FirstPerson);
    settings.set_rig(Some(CameraRig {
        roll_radians: f32::NAN,
        ..shoulder()
    }));
    assert_eq!(settings.rig(), None);
}

#[test]
fn rig_offset_follows_the_eye_look_in_camera_local_axes() {
    let eye = Vec3::new(10.0, 70.0, 10.0);
    let unturned = rig_pose(eye, Quat::IDENTITY, shoulder());
    assert!(
        unturned
            .translation
            .abs_diff_eq(Vec3::new(10.8, 70.4, 13.0), 1e-5)
    );
    assert_eq!(unturned.rotation, Quat::IDENTITY);
    // A quarter turn left moves "right" onto -Z and "back" onto +X.
    let turned = Quat::from_euler(EulerRot::YXZ, std::f32::consts::FRAC_PI_2, 0.0, 0.0);
    let pose = rig_pose(eye, turned, shoulder());
    assert!(
        pose.translation
            .abs_diff_eq(Vec3::new(13.0, 70.4, 9.2), 1e-4)
    );
    assert_eq!(pose.rotation, turned);
}

#[test]
fn rig_boom_stops_before_a_wall_behind_the_shoulder() {
    let eye = Vec3::new(0.0, 2.0, 0.0);
    let rig = CameraRig {
        offset: Vec3::new(0.0, 0.0, 3.0),
        ..shoulder()
    };
    let wall = Walls(vec![Aabb::new(
        SimVec3::new(-1.0, 1.0, 2.0),
        SimVec3::new(1.0, 3.0, 3.0),
    )]);
    let pose = collision_safe_rig_pose(eye, Quat::IDENTITY, rig, &wall);
    assert!(pose.translation.abs_diff_eq(
        Vec3::new(
            0.0,
            2.0,
            4.02_f32.sqrt() - THIRD_PERSON_COLLISION_EPSILON_BLOCKS
        ),
        1e-5
    ));
    let open = collision_safe_rig_pose(eye, Quat::IDENTITY, rig, &Walls(Vec::new()));
    assert!(open.translation.abs_diff_eq(Vec3::new(0.0, 2.0, 3.0), 1e-5));
}

#[test]
fn camera_corner_rays_leave_thin_geometry_between_the_rays_and_allocate_nothing() {
    let eye = Vec3::ZERO;
    let rig = CameraRig {
        offset: Vec3::new(0.0, 0.0, 4.0),
        ..shoulder()
    };
    let thin = Walls(vec![Aabb::new(
        SimVec3::new(-0.02, -0.02, 1.0),
        SimVec3::new(0.02, 0.02, 2.0),
    )]);
    assert_eq!(
        collision_safe_rig_pose(eye, Quat::IDENTITY, rig, &thin)
            .translation
            .z,
        4.0
    );
    let before = crate::test_allocations::count();
    for _ in 0..1000 {
        let _ = collision_safe_rig_pose(eye, Quat::IDENTITY, rig, &thin);
    }
    assert_eq!(crate::test_allocations::count() - before, 0);
}

#[test]
fn camera_collision_keeps_the_minimum_avoidance_distance() {
    let wall = Walls(vec![Aabb::new(
        SimVec3::new(-1.0, -1.0, 0.1),
        SimVec3::new(1.0, 1.0, 1.0),
    )]);
    let rig = CameraRig {
        offset: Vec3::new(0.0, 0.0, 4.0),
        ..shoulder()
    };
    let result = collision_safe_rig_pose(Vec3::ZERO, Quat::IDENTITY, rig, &wall);
    assert!((result.translation.z - 0.25).abs() < 1e-6);
}
