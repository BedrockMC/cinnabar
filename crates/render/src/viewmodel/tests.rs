use super::*;

#[test]
fn neutral_anchor_pins_all_independent_camera_local_corners() {
    let expected = [
        [0.40368183, -0.811_525_9, -0.503_502_2],
        [0.60918505, -0.92329095, -0.48905805],
        [0.668_541_3, -0.38853702, -0.99879473],
        [0.874_044_5, -0.50030211, -0.98435057],
        [0.333_642_1, -0.96172657, -0.66923036],
        [0.539_145_3, -1.073_491_7, -0.654_786_2],
        [0.59850155, -0.538_737_7, -1.164_522_9],
        [0.804_004_8, -0.650_502_8, -1.150_078_8],
    ];
    let transform = geometry::neutral_arm_transform();
    let mut index = 0;
    for x in [-3.0, 1.0] {
        for y in [-2.0, 10.0] {
            for z in [-2.0, 2.0] {
                let actual = transform.transform_point3(bevy::math::Vec3::new(x, y, z));
                for axis in 0..3 {
                    assert!((actual[axis] - expected[index][axis]).abs() < 0.000002);
                }
                index += 1;
            }
        }
    }
}

#[test]
fn completion_is_exact_and_invalidation_rejects_old_callbacks() {
    let gate = ViewmodelCompletionGate::default();
    let token = test_token();
    gate.select(Some(token));
    assert!(!gate.completed(token));
    let reservation = gate.reserve(token).unwrap();
    gate.select(None);
    gate.select(Some(token));
    assert!(!gate.complete(reservation));
    assert!(!gate.completed(token));
    let reservation = gate.reserve(token).unwrap();
    assert!(gate.complete(reservation));
    assert!(gate.completed(token));
    let mut resized = token;
    resized.viewport[0] += 1;
    gate.select(Some(resized));
    assert!(!gate.completed(resized));
}

#[test]
fn completed_gpu_hand_is_revoked_on_missing_coverage_or_epoch_exhaustion() {
    let gate = ViewmodelCompletionGate::default();
    let token = test_token();
    gate.select(Some(token));
    let reserved = gate.reserve(token).unwrap();
    assert!(gate.complete(reserved));
    gate.reject(token);
    assert!(!gate.completed(token));
    assert_eq!(gate.rejection_count(), 1);
    assert!(gate.rejected(token));
    let reserved = gate.reserve(token).unwrap();
    assert!(gate.complete(reserved));
    assert!(!gate.rejected(token));
    gate.0.lock().unwrap().epoch = u64::MAX;
    gate.select(None);
    gate.select(Some(token));
    assert!(gate.reserve(token).is_none());
    assert!(!gate.completed(token));
}

#[test]
fn token_lifetime_view_and_material_changes_each_require_fresh_completion() {
    let base = test_token();
    for changed in [
        ViewmodelToken { session: 2, ..base },
        ViewmodelToken {
            actor_session: 3,
            ..base
        },
        ViewmodelToken {
            dimension: -1,
            ..base
        },
        ViewmodelToken { spawn: 4, ..base },
        ViewmodelToken { samples: 4, ..base },
        ViewmodelToken { hdr: true, ..base },
        ViewmodelToken {
            skin: [7; 32],
            ..base
        },
        ViewmodelToken {
            geometry: [8; 32],
            ..base
        },
        ViewmodelToken {
            revision: 2,
            ..base
        },
    ] {
        let gate = ViewmodelCompletionGate::default();
        gate.select(Some(base));
        let reserved = gate.reserve(base).unwrap();
        gate.select(Some(changed));
        assert!(!gate.complete(reserved));
        assert!(!gate.completed(changed));
        assert!(gate.complete(gate.reserve(changed).unwrap()));
    }
}

#[test]
fn depth_limit_counts_samples_and_overflow() {
    assert_eq!(viewmodel_depth_bytes([1920, 1080], 4), Some(33_177_600));
    assert_eq!(viewmodel_depth_bytes([0, 1080], 1), None);
    assert_eq!(viewmodel_depth_bytes([u32::MAX, u32::MAX], 4), None);
    assert_eq!(viewmodel_depth_bytes([8192, 8192], 1), None);
}

#[test]
fn skin_fractional_alpha_is_not_silently_quantized() {
    let pixels: Arc<[u8]> = vec![255; 64 * 64 * 4].into();
    assert!(ViewmodelSkin::new(pixels.clone(), [1; 32]).is_some());
    let mut fractional = pixels.to_vec();
    fractional[3] = 128;
    assert!(ViewmodelSkin::new(fractional.into(), [2; 32]).is_none());
    assert!(ViewmodelSkin::new(Arc::from([255; 4]), [3; 32]).is_none());
}

fn test_token() -> ViewmodelToken {
    ViewmodelToken {
        session: 1,
        actor_session: 9,
        dimension: 0,
        runtime: 2,
        spawn: 3,
        owner: bevy::prelude::Entity::from_raw_u32(0).unwrap(),
        viewport: [1920, 1080],
        samples: 1,
        hdr: false,
        skin: [4; 32],
        geometry: [5; 32],
        revision: 1,
    }
}

fn profile() -> assets::EntityGeometry {
    use assets::{
        EntityGeometry, EntityGeometryBone, EntityGeometryCube, EntityGeometryScalar as S,
        EntityGeometryUv,
    };
    let vec = |value: [f32; 3]| value.map(|v| S::new(v).unwrap());
    let bones = [
        ("root", None, [0., 0., 0.]),
        ("waist", Some("root"), [0., 12., 0.]),
        ("body", Some("waist"), [0., 24., 0.]),
        ("rightArm", Some("body"), [-5., 22., 0.]),
        ("rightSleeve", Some("rightArm"), [-5., 22., 0.]),
    ]
    .into_iter()
    .map(|(name, parent, pivot)| {
        let cubes = if matches!(name, "rightArm" | "rightSleeve") {
            vec![EntityGeometryCube {
                origin: vec([-8., 12., -2.]),
                size: vec([4., 12., 4.]),
                pivot: vec([0.; 3]),
                rotation: vec([0.; 3]),
                uv: EntityGeometryUv::Box(
                    [40., if name == "rightArm" { 16. } else { 32. }].map(|v| S::new(v).unwrap()),
                ),
                inflate: S::new(if name == "rightArm" { 0. } else { 0.25 }).unwrap(),
                mirror: false,
            }]
        } else {
            Vec::new()
        };
        EntityGeometryBone {
            name: name.into(),
            parent: parent.map(Into::into),
            pivot: Some(vec(pivot)),
            rotation: None,
            inflate: None,
            mirror: None,
            never_render: None,
            reset: None,
            cubes: cubes.into(),
        }
    })
    .collect::<Vec<_>>()
    .into();
    EntityGeometry {
        identifier: "geometry.humanoid.custom".into(),
        inherits: None,
        source_index: 0,
        texture_width: 64,
        texture_height: 64,
        bones,
    }
}

#[test]
fn arm_and_sleeve_share_one_parent_transform_and_distinct_uv_faces() {
    let model = profile();
    let mesh = geometry::validated_geometry(&model, [5; 32]).unwrap();
    assert_eq!(mesh.vertices.len(), 72);
    let transform = geometry::neutral_arm_transform();
    assert_eq!(
        mesh.vertices[0].position,
        transform
            .transform_point3(bevy::math::Vec3::new(-3., 10., -2.))
            .to_array()
    );
    assert_eq!(mesh.vertices[0].uv, [44. / 64., 20. / 64.]);
    assert_eq!(mesh.vertices[1].uv, [48. / 64., 20. / 64.]);
    assert_eq!(mesh.vertices[2].uv, [48. / 64., 32. / 64.]);
    assert_eq!(mesh.vertices[36].uv, [44. / 64., 36. / 64.]);
    assert_eq!(
        mesh.vertices[36].position,
        transform
            .transform_point3(bevy::math::Vec3::new(-3.25, 10.25, -2.25))
            .to_array()
    );
    for face in mesh.vertices.chunks_exact(6) {
        assert_eq!(face[0].position, face[3].position);
        assert_eq!(face[2].position, face[4].position);
        assert_eq!(face[0].uv, face[3].uv);
        assert_eq!(face[2].uv, face[4].uv);
        let a = bevy::math::Vec3::from(face[1].position) - bevy::math::Vec3::from(face[0].position);
        let b = bevy::math::Vec3::from(face[2].position) - bevy::math::Vec3::from(face[0].position);
        assert!(a.cross(b).length_squared() > 0.);
        assert!(
            face.iter()
                .all(|vertex| vertex.position.iter().all(|v| v.is_finite())
                    && vertex.uv.iter().all(|v| (0.0..=1.0).contains(v)))
        );
    }
}

#[test]
fn unverified_profiles_reject_instead_of_recalibrating_the_anchor() {
    use assets::EntityGeometryScalar as S;
    let mut model = profile();
    model.bones[3].cubes[0].size[0] = S::new(3.).unwrap();
    assert!(geometry::validated_geometry(&model, [5; 32]).is_none());
    let mut model = profile();
    model.bones[2].rotation = Some([0., 0., 1.].map(|v| S::new(v).unwrap()));
    assert!(geometry::validated_geometry(&model, [5; 32]).is_none());
    let mut model = profile();
    model.bones[4].parent = Some("body".into());
    assert!(geometry::validated_geometry(&model, [5; 32]).is_none());
    let mut model = profile();
    model.texture_width = 128;
    assert!(geometry::validated_geometry(&model, [5; 32]).is_none());
    let mut model = profile();
    model.bones[3].never_render = Some(true);
    assert!(geometry::validated_geometry(&model, [5; 32]).is_none());
}

#[test]
fn reverse_z_projection_is_private_aspect_correct_and_world_fov_independent() {
    let projection = hand_projection([1920, 1080]);
    let near = projection.project_point3(bevy::math::Vec3::new(0., 0., -0.1));
    assert!((near.z - 1.).abs() < 0.000001);
    let far = projection.project_point3(bevy::math::Vec3::new(0., 0., -1000.));
    assert!(far.z > 0. && far.z < 0.001);
    assert!((projection.y_axis.y / projection.x_axis.x - 1920. / 1080.).abs() < 0.000001);
}

fn fallback_input() -> crate::ui::UiRenderInput {
    use crate::ui::*;
    UiRenderInput {
        revision: 1,
        viewport_size: test_token().viewport,
        safe_area: [0; 4],
        vertices: [
            ([1., 2.], [4, 8]),
            ([3., 2.], [12, 8]),
            ([3., 4.], [12, 24]),
            ([1., 4.], [4, 24]),
        ]
        .map(|(position, uv)| UiRenderVertex {
            position,
            uv,
            color: [255; 4],
            style_flags: 0,
        })
        .into(),
        indices: Arc::from([0, 1, 2, 0, 2, 3]),
        batches: Arc::from([UiRenderBatch::new(
            0,
            UiScissor::new(0, 0, 1920, 1080),
            0,
            6,
            UI_BLEND_ALPHA,
        )]),
        textures: Arc::new(
            crate::UiTextureCatalog::new(
                vec![crate::UiTexturePage::owned([64, 64], vec![255; 64 * 64 * 4].into()).unwrap()],
                1,
            )
            .unwrap(),
        ),
    }
}
fn fallback_scene(gate: &ViewmodelCompletionGate) -> ViewmodelScene {
    let mut scene = ViewmodelScene::default();
    let skin = ViewmodelSkin::new(vec![255; 64 * 64 * 4].into(), [4; 32]).unwrap();
    let geometry = geometry::validated_geometry(&profile(), [5; 32]).unwrap();
    assert!(scene.publish(test_token(), &skin, &geometry, gate));
    scene
}
#[test]
fn cpu_fallback_join_is_unique_bounded_and_ui_revision_does_not_reset_lifetime_completion() {
    let gate = ViewmodelCompletionGate::default();
    let mut scene = fallback_scene(&gate);
    let mut input = fallback_input();
    assert!(scene.bind_cpu_fallback(&input, 0, [4, 8, 12, 24], &gate));
    assert_eq!(scene.frame.as_ref().unwrap().fallback, Some((1, 0, 0)));
    assert!(gate.complete(gate.reserve(test_token()).unwrap()));
    input.revision = 2;
    assert!(scene.bind_cpu_fallback(&input, 0, [4, 8, 12, 24], &gate));
    assert!(gate.completed(test_token()));
    assert_eq!(scene.frame.as_ref().unwrap().fallback, Some((2, 0, 0)));
    for hostile in [0, 1, 2, 3] {
        let mut scene = fallback_scene(&gate);
        let mut input = fallback_input();
        match hostile {
            0 => {
                input.indices = Arc::from([0, 1, 2, 0, 2, 3, 0, 1, 2, 0, 2, 3]);
                let mut batch = input.batches[0];
                batch.index_count = 12;
                input.batches = Arc::from([batch]);
            }
            1 => input.indices = Arc::from([0, 1, 2, 0, 2, 99]),
            2 => input.batches = Arc::from([input.batches[0], input.batches[0]]),
            _ => input.viewport_size[0] += 1,
        }
        assert!(!scene.bind_cpu_fallback(&input, 0, [4, 8, 12, 24], &gate));
        assert!(scene.frame.is_none());
        assert!(!gate.completed(test_token()));
    }
}

#[test]
fn actual_renderer_startup_revokes_pending_callback_and_equal_token_publish_recovers() {
    use bevy::{
        app::SubApp,
        ecs::schedule::Schedule,
        render::{ExtractSchedule, Render, RenderApp, RenderStartup},
    };
    let (device, _) = wgpu::Device::noop(&wgpu::DeviceDescriptor::default());
    let mut render_app = SubApp::new();
    render_app
        .insert_resource(bevy::render::renderer::RenderDevice::from(device))
        .add_schedule(Schedule::new(RenderStartup))
        .add_schedule(Render::base_schedule())
        .add_schedule(Schedule::new(ExtractSchedule));
    let mut app = App::new();
    app.insert_resource(Assets::<Shader>::default())
        .insert_sub_app(RenderApp, render_app);
    app.add_plugins(crate::viewmodel_render::ViewmodelRenderPlugin);
    app.finish();
    let gate = app.world().resource::<ViewmodelCompletionGate>().clone();
    let mut scene = fallback_scene(&gate);
    let token = test_token();
    let old = gate.reserve(token).unwrap();
    app.sub_app_mut(RenderApp)
        .world_mut()
        .run_schedule(RenderStartup);
    assert!(!gate.complete(old));
    assert!(gate.reserve(token).is_none());
    assert!(!gate.completed(token));
    let frame = scene.frame.as_ref().unwrap().clone();
    assert!(scene.publish(token, &frame.skin, &frame.geometry, &gate));
    assert!(!gate.complete(old));
    let fresh = gate.reserve(token).unwrap();
    assert!(!gate.complete(old));
    assert!(gate.complete(fresh));
    assert!(gate.completed(token));
}

#[test]
fn fallback_identity_is_logical_even_when_its_layer_is_in_another_bucket() {
    let gate = ViewmodelCompletionGate::default();
    let mut scene = fallback_scene(&gate);
    let mut input = fallback_input();
    input.textures = Arc::new(
        crate::UiTextureCatalog::new(
            vec![
                crate::UiTexturePage::owned([1024, 1024], vec![255; 1024 * 1024 * 4].into())
                    .unwrap(),
                crate::UiTexturePage::owned([64, 64], vec![255; 64 * 64 * 4].into()).unwrap(),
            ],
            2,
        )
        .unwrap(),
    );
    let mut batch = input.batches[0];
    batch.texture_page = 1;
    input.batches = Arc::from([batch]);
    assert_eq!(
        input.textures.plan().locations()[1],
        crate::UiTextureLocation {
            bucket: 1,
            layer: 0
        }
    );
    assert!(scene.bind_cpu_fallback(&input, 1, [4, 8, 12, 24], &gate));
    assert_eq!(scene.frame.as_ref().unwrap().fallback, Some((1, 0, 1)));
    assert!(!scene.bind_cpu_fallback(&input, 0, [4, 8, 12, 24], &gate));
}
