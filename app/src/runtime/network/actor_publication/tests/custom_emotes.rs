//! Original geometry through the production frame preparation system, without GPU/assets.
use std::{sync::Arc, time::Duration};

use bevy::{prelude::*, time::Real};
use protocol::{ActorEvent, ActorKind, ActorSpawnEvent, WorldBootstrap, WorldEvent};
use semantic_input::PerspectiveMode;

use crate::{
    runtime::{network::actor_publication::*, world::ClientWorld},
    ui_runtime::UiRuntime,
};

fn fixture() -> World {
    let entity = br#"{"format_version":"1.10.0","minecraft:client_entity":{"description":{"identifier":"minecraft:player","materials":{"default":"entity"},"textures":{"default":"textures/entity/test_player"},"geometry":{"default":"geometry.test_player"},"render_controllers":["controller.render.test_player"]}}}"#;
    let geometry = br#"{"format_version":"1.12.0","minecraft:geometry":[{"description":{"identifier":"geometry.test_player","texture_width":64,"texture_height":64},"bones":[
        {"name":"root","pivot":[0,0,0]},
        {"name":"waist","parent":"root","pivot":[0,12,0]},
        {"name":"body","parent":"waist","pivot":[0,24,0],"cubes":[{"origin":[-4,12,-2],"size":[8,12,4],"uv":[16,16]}]},
        {"name":"head","parent":"body","pivot":[0,24,0]},
        {"name":"leftArm","parent":"body","pivot":[5,22,0],"cubes":[{"origin":[4,12,-2],"size":[4,12,4],"uv":[32,48]}]},
        {"name":"rightArm","parent":"body","pivot":[-5,22,0],"cubes":[{"origin":[-8,12,-2],"size":[4,12,4],"uv":[40,16]}]},
        {"name":"leftLeg","parent":"root","pivot":[2,12,0]},
        {"name":"rightLeg","parent":"root","pivot":[-2,12,0]}]}]}"#;
    let controller = br#"{"format_version":"1.8.0","render_controllers":{"controller.render.test_player":{"geometry":"Geometry.default","materials":[{"*":"Material.default"}],"textures":["Texture.default"]}}}"#;
    let compiled = pack_compiler::compile_entity_pack(vec![
        ("entity/player.json".into(), entity.to_vec()),
        ("models/entity/player.geo.json".into(), geometry.to_vec()),
        ("render_controllers/player.json".into(), controller.to_vec()),
    ])
    .unwrap()
    .unwrap();
    let entities = Arc::new(assets::RuntimeEntityAssets::from_compiled(compiled.assets).unwrap());
    let assets = Arc::new(assets::RuntimeAssets::diagnostic());
    let anchor = [0.0, 64.0 + protocol::PLAYER_NETWORK_OFFSET, 0.0];
    let mut stream = client_world::WorldStream::new_with_asset_sets(
        WorldBootstrap {
            dimension: 0,
            local_player_runtime_id: 1,
            local_player_unique_id: 1,
            player_position: anchor,
            world_spawn_position: [0, 64, 0],
            air_network_id: 0,
            block_network_ids_are_hashes: false,
        },
        assets.clone(),
        entities.clone(),
        anchor,
        None,
    );
    for (runtime_id, position) in [(1, [0.0, 64.0, 0.0]), (2, [2.0, 64.0, 0.0])] {
        stream
            .submit(
                runtime_id,
                WorldEvent::Actor(ActorEvent::Spawn(ActorSpawnEvent {
                    dimension: 0,
                    unique_id: runtime_id as i64,
                    runtime_id,
                    kind: ActorKind::Player {
                        uuid: [runtime_id as u8; 16],
                        username: "fixture".into(),
                    },
                    position,
                    velocity: [0.0; 3],
                    pitch: 0.0,
                    yaw: 0.0,
                    head_yaw: 0.0,
                    body_yaw: 0.0,
                    held_item: Default::default(),
                    metadata: Arc::from([]),
                    attributes: Arc::from([]),
                    properties: Arc::from([]),
                    links: Arc::from([]),
                })),
            )
            .unwrap();
    }
    let mut client = ClientWorld::new_with_entity_assets(assets, entities.clone());
    client.stream = Some(stream);
    let mut physics = crate::movement::LocalPhysicsController::default();
    physics.reanchor_network_position(anchor, 0, true);
    let mut avatar = crate::local_player::LocalAvatarPresentation::default();
    avatar.begin_session(1, 1);
    let icons = Arc::new(
        assets::RuntimeIconCatalog::decode(
            &assets::encode_icon_catalog([0; 32], &[], &[]).unwrap(),
        )
        .unwrap(),
    );
    let (equipment, artwork, _) = crate::presentation::equipment::EquipmentRuntime::build(
        entities.clone(),
        None,
        icons,
        None,
        None,
        render::ActorArtworkPages::default(),
    );
    let mut world = World::new();
    world.insert_resource(crate::player_runtime::PlayerRuntime::new(1));
    world.insert_resource(client);
    world.insert_resource(Time::<Real>::default());
    world.insert_resource(render::ActorRenderScene::with_runtime_entity_assets(&entities).unwrap());
    world.insert_resource(PreparedActorPublication::default());
    world.insert_resource(avatar);
    world.init_resource::<crate::local_player::LocalAvatarVisibilityCarrier>();
    world.init_resource::<crate::camera::CameraSettingsAuthority>();
    world.insert_resource(physics);
    world.insert_resource(artwork);
    world.insert_resource(equipment);
    world.insert_resource(HandRigBuilder::from_runtime_assets(&entities).unwrap());
    world.init_resource::<render::HandRigScene>();
    world.insert_resource(crate::player_skin::LocalPlayerSkin::generated_default(
        "emote fixture",
    ));
    world.init_resource::<ActorFramePartialTick>();
    world.insert_resource(UiRuntime::new(1));
    let camera =
        Transform::from_xyz(0.0, 66.0, -8.0).looking_at(Vec3::new(0.0, 65.0, 0.0), Vec3::Y);
    world.spawn((
        camera,
        Projection::Perspective(PerspectiveProjection::default()),
        crate::camera::FlyCamera::default(),
    ));
    world.insert_resource(crate::local_player::LocalViewPose::new(
        Vec3::from_array(anchor),
        camera.rotation,
    ));
    world
}

fn perspective(world: &mut World, mode: PerspectiveMode, generation: u64) {
    let mut settings = ui::UserSettings::default();
    settings.gameplay.default_perspective = mode;
    world
        .resource_mut::<crate::camera::CameraSettingsAuthority>()
        .replace(generation, &settings)
        .unwrap();
}

fn prepare(world: &mut World, millis: u64) {
    world
        .resource_mut::<Time<Real>>()
        .advance_by(Duration::from_millis(millis));
    world.run_system_cached(prepare_actor_render_frame).unwrap();
}

fn body(world: &World, id: u64) -> render::ActorRigSubmission {
    world
        .resource::<PreparedActorPublication>()
        .0
        .as_ref()
        .unwrap()
        .batch
        .submissions
        .iter()
        .find(|body| {
            body.input.identity.runtime_id == id
                && body.input.identity.layer == render::ACTOR_LAYER_BODY
        })
        .expect("visible production body")
        .clone()
}

fn native_pose(world: &World) -> (u64, Vec<client_world::BoneTransform>) {
    let stream = world.resource::<ClientWorld>().stream.as_ref().unwrap();
    let rig = stream.actor_rig(1).unwrap();
    (rig.completed_tick, rig.current.to_vec())
}

#[test]
fn custom_emote_publication_advances_between_ticks_without_changing_native_hand_or_remote_pose() {
    let mut world = fixture();
    perspective(&mut world, PerspectiveMode::ThirdPersonBack, 1);
    prepare(&mut world, 100);
    let native = native_pose(&world);
    let rest = body(&world, 1);
    {
        let mut ui = world.resource_mut::<UiRuntime>();
        ui.emotes_mut().open();
        assert_eq!(
            ui.emotes_mut().activate_slot(0, 100),
            Some(client_world::CustomEmote::Twerk)
        );
    }
    prepare(&mut world, 10);
    let first = body(&world, 1);
    let remote = body(&world, 2);
    assert_ne!(first.input.current_bones, rest.input.current_bones);
    assert_eq!(first.input.previous_bones, first.input.current_bones);
    prepare(&mut world, 10);
    let second = body(&world, 1);
    assert_eq!(
        native_pose(&world),
        native,
        "render sampling leaves the tick-owned native rig untouched"
    );
    assert_ne!(
        first.input.current_bones, second.input.current_bones,
        "a held actor tick must not freeze the dance"
    );
    assert_eq!(
        body(&world, 2).input,
        remote.input,
        "remote rig never receives the local overlay"
    );
    assert_eq!(first.input.rig, second.input.rig);
    assert_eq!(first.world_from_actor, second.world_from_actor);
    perspective(&mut world, PerspectiveMode::FirstPerson, 2);
    prepare(&mut world, 0);
    assert!(
        world.resource::<render::HandRigScene>().is_active(),
        "native hand still publishes during playback"
    );
    assert_eq!(native_pose(&world), native);
    assert!(
        world
            .resource::<PreparedActorPublication>()
            .0
            .as_ref()
            .unwrap()
            .batch
            .submissions
            .iter()
            .all(|body| body.input.identity.runtime_id != 1),
        "first person does not publish the custom body"
    );
    world.resource_mut::<UiRuntime>().emotes_mut().stop();
    prepare(&mut world, 0);
    assert!(world.resource::<render::HandRigScene>().is_active());
    assert_eq!(native_pose(&world), native);
    assert_eq!(body(&world, 2).input, remote.input);
}
