use super::*;
use crate::presentation::equipment::HeldKind;
use bevy::math::Quat;
use render::ActorArtworkPages;

/// Supplies an original attack-driven player arm and an identity-pose held attachable.
fn fixture(
    main: Option<&str>,
) -> (
    WorldStream,
    EquipmentRuntime,
    ActorArtworkPages,
    ActorEquipmentInput,
) {
    let geometry = serde_json::json!({"format_version":"1.12.0","minecraft:geometry":[{
        "description":{"identifier":"geometry.player_test","texture_width":64,"texture_height":64},
        "bones":[{"name":"head","pivot":[0,24,0]}, {"name":"body","pivot":[0,24,0]},
            {"name":"rightarm","pivot":[4,22,0],"cubes":[{"origin":[4,12,0],"size":[4,12,4],"uv":[0,0]}]},
            {"name":"leftarm","pivot":[-4,22,0]}, {"name":"rightleg","pivot":[2,12,0]},
            {"name":"leftleg","pivot":[-2,12,0]}, {"name":"rightItem","parent":"rightarm","pivot":[4,12,0]},
            {"name":"leftItem","parent":"leftarm","pivot":[-4,12,0]}]
    }]});
    let mut png = std::io::Cursor::new(Vec::new());
    image::RgbaImage::from_pixel(16, 16, image::Rgba([255; 4]))
        .write_to(&mut png, image::ImageFormat::Png)
        .unwrap();
    let files: Vec<(Box<str>, Vec<u8>)> = vec![
        ("entity/player.json".into(), br#"{"format_version":"1.10.0","minecraft:client_entity":{"description":{"identifier":"minecraft:player","materials":{"default":"entity"},"textures":{"default":"textures/test"},"geometry":{"default":"geometry.player_test"},"animations":{"swing":"animation.player_test.swing"},"scripts":{"animate":["swing"]},"render_controllers":["controller.render.test"]}}}"#.to_vec()),
        ("models/entity/player.geo.json".into(), geometry.to_string().into_bytes()),
        ("animations/player.animation.json".into(), br#"{"format_version":"1.8.0","animations":{"animation.player_test.swing":{"loop":true,"bones":{"rightarm":{"rotation":["variable.attack_time * 80.0",0,0]},"leftarm":{"rotation":["variable.attack_time * 60.0",0,0]}}}}}"#.to_vec()),
        ("models/entity/item.geo.json".into(), br#"{"format_version":"1.12.0","minecraft:geometry":[{"description":{"identifier":"geometry.item_test","texture_width":16,"texture_height":16},"bones":[{"name":"item","pivot":[0,0,0],"cubes":[{"origin":[0,0,0],"size":[1,1,1],"uv":[0,0]}]}]}]}"#.to_vec()),
        ("attachables/shield.json".into(), br#"{"format_version":"1.10.0","minecraft:attachable":{"description":{"identifier":"minecraft:shield","materials":{"default":"entity_alphatest"},"textures":{"default":"textures/test"},"geometry":{"default":"geometry.item_test"},"render_controllers":["controller.render.test"]}}}"#.to_vec()),
        ("render_controllers/test.json".into(), br#"{"format_version":"1.8.0","render_controllers":{"controller.render.test":{"geometry":"Geometry.default","materials":[{"*":"Material.default"}],"textures":["Texture.default"]}}}"#.to_vec()),
        ("textures/test.png".into(), png.into_inner()),
    ];
    let mut files = files;
    let bow = files
        .iter()
        .find(|(path, _)| &**path == "attachables/shield.json")
        .unwrap()
        .1
        .clone();
    files.push((
        "attachables/bow.json".into(),
        String::from_utf8(bow)
            .unwrap()
            .replace("minecraft:shield", "minecraft:bow")
            .into_bytes(),
    ));
    let compiled = pack_compiler::compile_actor_pack(files).unwrap().unwrap();
    let catalog = Arc::new(
        assets::RuntimeEquipmentCatalog::from_parts(
            compiled.identity,
            compiled.equipment_bindings,
            compiled.equipment_textures,
        )
        .unwrap(),
    );
    let entities = Arc::new(assets::RuntimeEntityAssets::from_compiled(compiled.entities).unwrap());
    let sprite = assets::IconSprite {
        width: 16,
        height: 16,
        rgba8: vec![255; 16 * 16 * 4].into(),
    };
    let icons = Arc::new(
        assets::RuntimeIconCatalog::decode(
            &assets::encode_icon_catalog(
                entities.source_manifest_sha256(),
                &[sprite],
                &[
                    "minecraft:bow",
                    "minecraft:diamond_sword",
                    "minecraft:shield",
                ]
                .map(|identifier| assets::IconEntry {
                    identifier: identifier.into(),
                    metadata: 0,
                    sprite: 0,
                }),
            )
            .unwrap(),
        )
        .unwrap(),
    );
    let (equipment, artwork, _) = EquipmentRuntime::build(
        entities.clone(),
        Some(catalog),
        icons,
        None,
        None,
        ActorArtworkPages::default(),
    );
    let mut stream = WorldStream::new_with_asset_sets(
        protocol::WorldBootstrap {
            dimension: 0,
            local_player_runtime_id: 1,
            local_player_unique_id: 1,
            player_position: [0.0, 64.0, 0.0],
            world_spawn_position: [0, 64, 0],
            air_network_id: protocol::SEQUENTIAL_AIR_NETWORK_ID,
            block_network_ids_are_hashes: false,
        },
        Arc::new(assets::RuntimeAssets::diagnostic()),
        entities,
        [0.0, 64.0, 0.0],
        None,
    );
    let mut feed = client_world::LocalPlayerFeed {
        prefer_client_skin: false,
        uuid: [1; 16],
        username: Arc::from("Player"),
        skin: protocol::PlayerSkin::Unavailable(protocol::PlayerSkinUnavailable::InvalidDimensions),
        position: [0.0, 64.0, 0.0],
        velocity: [0.0; 3],
        on_ground: true,
        yaw: 170.0,
        head_yaw: 170.0,
        pitch: 5.0,
        main_hand: None,
        off_hand: None,
        main_hand_metadata: 0,
        main_hand_slot: 0,
        main_hand_stack_id: None,
        bedrock_swing_ticks: client_world::ACTOR_SWING_TICKS,
        java_swing_ticks: client_world::ACTOR_SWING_TICKS,
        flying: false,
        teleported: false,
        first_person: false,
        view_bobbing: true,
        sneaking: false,
        sprinting: false,
        item_use: client_world::LocalItemUse::Unpredicted,
    };
    feed.first_person = true;
    feed.main_hand = main.map(Arc::from);
    stream.sync_local_player_pose(&feed);
    stream.advance_actor_interpolation_frame(6);
    let input = ActorEquipmentInput {
        main: main.map(|identifier| crate::presentation::equipment::WornItem {
            identifier: identifier.into(),
            metadata: 0,
            damage: None,
            dye_rgb: None,
            enchanted: false,
            kind: HeldKind::Sprite,
        }),
        ..Default::default()
    };
    (stream, equipment, artwork, input)
}

/// Uses the production selector with independent actor and physics fractions.
fn hand_source_for_mode(
    stream: &WorldStream,
    equipment: &mut EquipmentRuntime,
    artwork: &ActorArtworkPages,
    input: &ActorEquipmentInput,
    cache: &mut java::HandCache,
    actor_alpha: f32,
    java_mode: bool,
) -> HandSource {
    let rig = stream.authority().actor_rig(1).unwrap();
    let actor = stream.authority().actor(1).unwrap();
    let presentation = crate::presentation::actors::actor_rig_presentation(
        &rig,
        actor,
        stream.authority().actor_player_profile(1),
        actor_alpha,
    )
    .unwrap();
    source(
        HandInputs {
            stream,
            presentation,
            equipment_input: input,
            owner_equipment: input,
            consume_ticks: None,
            item_animation: Some(client_world::AttachableAnimationInput {
                first_person: true,
                ..Default::default()
            }),
            alpha: actor_alpha,
            artwork,
            motion: Mat4::IDENTITY,
            sampling_camera: Some(([0.0; 2], [0.0; 3])),
        },
        java_mode,
        equipment,
        cache,
    )
    .unwrap()
}

/// Checks native arm and attachable parent rotations against the sampled attack expression.
fn assert_sampled(held: bool) {
    let (mut stream, mut equipment, artwork, input) = fixture(held.then_some("minecraft:shield"));
    let mut cache = java::HandCache::default();
    for (physics_alpha, actor_alpha) in [(0.75, 0.1), (0.25, 0.9)] {
        stream.sync_local_swing(client_world::LocalSwingProgress {
            bedrock: [0.25, 0.5],
            java: [0.25, 0.5],
            frame_alpha: Some(physics_alpha),
        });
        stream.advance_actor_interpolation_frame(0);
        let rig = stream.authority().actor_rig(1).unwrap();
        let tick = rig.completed_tick;
        let previous = rig.previous.to_vec();
        let current = rig.current.to_vec();
        let stats = stream.authority().actor_animation_stats();
        let expected = Quat::from_rotation_x(-(0.25 + 0.25 * physics_alpha) * 80f32.to_radians());
        let result = hand_source_for_mode(
            &stream,
            &mut equipment,
            &artwork,
            &input,
            &mut cache,
            actor_alpha,
            false,
        );
        let (label, actual) = if held {
            let item = &result.items[0].as_ref().expect("native attachable").0;
            (
                "native attachable parent",
                item.presentation.submission.input.current_bones[0],
            )
        } else {
            let arm = rig
                .bone_names
                .iter()
                .position(|name| name.as_ref() == "rightarm")
                .unwrap();
            (
                "native arm",
                result
                    .body
                    .as_ref()
                    .expect("native arm")
                    .input
                    .current_bones[arm],
            )
        };
        assert!(
            Quat::from_array(actual.rotation).abs_diff_eq(expected, 1e-5),
            "{label} must sample physics alpha {physics_alpha}, not actor alpha {actor_alpha}: {:?}",
            actual.rotation
        );
        let unchanged = stream.authority().actor_rig(1).unwrap();
        assert_eq!(unchanged.completed_tick, tick);
        assert_eq!(unchanged.previous, previous);
        assert_eq!(unchanged.current, current);
        assert_eq!(stream.authority().actor_animation_stats(), stats);
    }
}

#[test]
fn native_hand_arm_samples_the_physics_swing_without_mutating_committed_poses() {
    assert_sampled(false);
}

#[test]
fn native_hand_attachable_parent_samples_the_physics_swing_without_mutating_committed_poses() {
    assert_sampled(true);
}

#[test]
fn native_hand_unchanged_sampling_reuses_the_owned_pose_without_work() {
    let (mut stream, mut equipment, artwork, input) = fixture(None);
    stream.sync_local_swing(client_world::LocalSwingProgress {
        bedrock: [0.25, 0.5],
        java: [0.25, 0.5],
        frame_alpha: Some(0.75),
    });
    stream.advance_actor_interpolation_frame(0);
    let mut cache = java::HandCache::default();
    let result = hand_source_for_mode(
        &stream,
        &mut equipment,
        &artwork,
        &input,
        &mut cache,
        0.1,
        false,
    );
    let rig = stream.authority().actor_rig(1).unwrap();
    let presentation = crate::presentation::actors::actor_rig_presentation(
        &rig,
        stream.authority().actor(1).unwrap(),
        stream.authority().actor_player_profile(1),
        0.1,
    )
    .unwrap();
    let animation = Some(client_world::AttachableAnimationInput {
        first_person: true,
        ..Default::default()
    });
    let stats = stream.authority().actor_animation_stats();
    let allocated = crate::test_allocations::count();
    let [previous, current] = cache
        .native_pose
        .sample(
            &stream,
            &presentation,
            None,
            animation,
            0.1,
            Some(([0.0; 2], [0.0; 3])),
        )
        .expect("retained sampled native pose");
    assert_eq!(
        crate::test_allocations::count() - allocated,
        0,
        "unchanged sampling must reuse its conversion"
    );
    assert!(Arc::ptr_eq(
        &previous,
        &result.presentation.submission.input.previous_bones
    ));
    assert!(Arc::ptr_eq(
        &current,
        &result.presentation.submission.input.current_bones
    ));
    assert_eq!(stream.authority().actor_animation_stats(), stats);
}

/// Exercises fraction changes after Java's ordinary draw and rest-arm caches are warm.
fn assert_java_without_native_work(main: Option<&str>) {
    let (mut stream, mut equipment, artwork, input) = fixture(main);
    let mut cache = java::HandCache::default();
    for physics_alpha in [0.75, 0.25] {
        stream.sync_local_swing(client_world::LocalSwingProgress {
            bedrock: [0.25, 0.5],
            java: [0.25, 0.5],
            frame_alpha: Some(physics_alpha),
        });
        stream.advance_actor_interpolation_frame(0);
        let rig = stream.authority().actor_rig(1).unwrap();
        cache.remember(&rig, input.main.as_ref());
        let result = hand_source_for_mode(
            &stream,
            &mut equipment,
            &artwork,
            &input,
            &mut cache,
            0.1,
            true,
        );
        assert_eq!(result.body.is_some(), main.is_none());
        assert_eq!(result.items[0].is_some(), main.is_some());
        assert_eq!(
            cache.native_pose.sample_work,
            (0, 0),
            "Java rest arms and camera-space items must not evaluate or allocate native sampled poses"
        );
    }
}

#[test]
fn native_hand_java_empty_arm_never_samples_discarded_native_attack_poses() {
    assert_java_without_native_work(None);
}

#[test]
fn native_hand_java_ordinary_item_never_samples_discarded_native_attack_poses() {
    assert_java_without_native_work(Some("minecraft:diamond_sword"));
}

/// Checks Java paths that still compose authored attachables on a sampled player parent.
fn assert_java_native_parent(main: Option<&str>, off: bool, degrees: f32) {
    let (mut stream, mut equipment, artwork, mut input) = fixture(main);
    if off {
        input.off = Some(crate::presentation::equipment::WornItem {
            identifier: "minecraft:shield".into(),
            metadata: 0,
            damage: None,
            dye_rgb: None,
            enchanted: false,
            kind: HeldKind::Sprite,
        });
    }
    let mut cache = java::HandCache::default();
    for physics_alpha in [0.75, 0.25] {
        stream.sync_local_swing(client_world::LocalSwingProgress {
            bedrock: [0.25, 0.5],
            java: [0.25, 0.5],
            frame_alpha: Some(physics_alpha),
        });
        stream.advance_actor_interpolation_frame(0);
        let rig = stream.authority().actor_rig(1).unwrap();
        cache.remember(&rig, input.main.as_ref());
        let result = hand_source_for_mode(
            &stream,
            &mut equipment,
            &artwork,
            &input,
            &mut cache,
            0.1,
            true,
        );
        let item = &result.items[usize::from(off)]
            .as_ref()
            .expect("native Java fallback attachable")
            .0;
        assert!(
            !item.camera_space,
            "original cube attachable uses native parent composition"
        );
        let expected = Quat::from_rotation_x(-(0.25 + 0.25 * physics_alpha) * degrees.to_radians());
        assert!(
            Quat::from_array(item.presentation.submission.input.current_bones[0].rotation)
                .abs_diff_eq(expected, 1e-5)
        );
        assert!(cache.native_pose.sample_work.0 > 0);
        assert!(cache.native_pose.sample_work.1 > 0);
    }
}

#[test]
fn native_hand_java_vanilla_draws_keep_the_sampled_parent() {
    assert_java_native_parent(Some("minecraft:shield"), false, 80.0);
}

#[test]
fn native_hand_java_offhand_attachables_keep_the_sampled_parent() {
    assert_java_native_parent(None, true, 60.0);
}

#[test]
fn native_hand_java_bow_raster_fallback_keeps_the_sampled_parent() {
    assert_java_native_parent(Some("minecraft:bow"), false, 80.0);
}
