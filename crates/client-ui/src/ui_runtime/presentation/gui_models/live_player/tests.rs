use super::*;
use std::sync::Arc;

/// Compiles original one-cube player geometry without installed assets or a server.
fn player_catalog(size: u32) -> Arc<assets::RuntimeEntityAssets> {
    let geometry = format!(
        r#"{{"format_version":"1.12.0","minecraft:geometry":[{{"description":{{"identifier":"geometry.test_player","texture_width":64,"texture_height":64}},"bones":[{{"name":"body","pivot":[0,0,0],"cubes":[{{"origin":[0,0,0],"size":[{size},8,4],"uv":[0,0]}}]}}]}}]}}"#
    );
    player_catalog_geometry(geometry.into_bytes())
}

fn player_catalog_geometry(geometry: Vec<u8>) -> Arc<assets::RuntimeEntityAssets> {
    let entity = br#"{"format_version":"1.10.0","minecraft:client_entity":{"description":{"identifier":"minecraft:player","materials":{"default":"entity"},"textures":{"default":"textures/entity/test_player"},"geometry":{"default":"geometry.test_player"},"render_controllers":["controller.render.test_player"]}}}"#;
    let render = br#"{"format_version":"1.8.0","render_controllers":{"controller.render.test_player":{"geometry":"Geometry.default","materials":[{"*":"Material.default"}],"textures":["Texture.default"]}}}"#;
    let compiled = pack_compiler::compile_entity_pack(vec![
        ("entity/player.json".into(), entity.to_vec()),
        ("models/entity/player.geo.json".into(), geometry),
        ("render_controllers/player.json".into(), render.to_vec()),
    ])
    .unwrap()
    .expect("original fixture compiles");
    assert_eq!(compiled.assets.rig_bindings.len(), 1, "player rig compiles");
    Arc::new(assets::RuntimeEntityAssets::from_compiled(compiled.assets).unwrap())
}

#[test]
fn hud_local_emote_moves_vertices_and_equipment_without_replacing_geometry_or_native_pose() {
    let catalog = player_catalog_geometry(br#"{"format_version":"1.12.0","minecraft:geometry":[{
        "description":{"identifier":"geometry.test_player","texture_width":64,"texture_height":64},"bones":[
        {"name":"root","pivot":[0,0,0]},
        {"name":"waist","parent":"root","pivot":[0,11,0]},
        {"name":"body","parent":"waist","pivot":[0,22,0],"cubes":[{"origin":[-3,11,-2],"size":[6,11,4],"uv":[0,0]}]},
        {"name":"head","parent":"body","pivot":[0,24,0]},
        {"name":"leftArm","parent":"body","pivot":[4,20,0]},
        {"name":"rightArm","parent":"body","pivot":[-4,20,0]},
        {"name":"leftLeg","parent":"root","pivot":[2,11,0]},
        {"name":"rightLeg","parent":"root","pivot":[-2,11,0]}]}]}"#.to_vec());
    let stream = player_stream(Arc::clone(&catalog), catalog);
    let native = stream
        .actor_ui_pose(stream.local_player_runtime_id())
        .unwrap()
        .to_vec();
    let mut presentation =
        UiPresentationRuntime::new(super::super::super::tests::fixture_font()).unwrap();
    presentation.capture_hud_player(Some(&stream), false);
    let rest = presentation.gui_models.live_player.vertices.clone();
    let geometry = presentation
        .gui_models
        .live_player
        .geometry
        .as_ref()
        .unwrap()
        .vertices
        .as_ptr();
    let emote = client_world::CustomEmote::Twerk;
    presentation.capture_hud_player_with_emote(Some(&stream), false, Some((emote, 0.0)));
    let first = presentation.gui_models.live_player.vertices.clone();
    assert_ne!(first, rest);
    assert!(
        presentation
            .gui_models
            .live_player
            .parts
            .iter()
            .all(Option::is_some)
    );
    presentation.capture_hud_player_with_emote(
        Some(&stream),
        false,
        Some((emote, emote.duration_seconds() / 4.0)),
    );
    assert_ne!(presentation.gui_models.live_player.vertices, first);
    assert_eq!(
        presentation
            .gui_models
            .live_player
            .geometry
            .as_ref()
            .unwrap()
            .vertices
            .as_ptr(),
        geometry
    );
    assert_eq!(
        stream
            .actor_ui_pose(stream.local_player_runtime_id())
            .unwrap(),
        native
    );
    presentation.capture_hud_player_with_emote(
        Some(&stream),
        false,
        Some((emote, emote.duration_seconds())),
    );
    assert_eq!(presentation.gui_models.live_player.vertices, first);
    presentation.capture_hud_player_with_emote(Some(&stream), false, None);
    assert_eq!(presentation.gui_models.live_player.vertices, rest);
}

/// Spawns an offline player whose rig comes from the supplied session catalog.
fn player_stream(
    vanilla: Arc<assets::RuntimeEntityAssets>,
    pack: Arc<assets::RuntimeEntityAssets>,
) -> client_world::WorldStream {
    let mut stream = client_world::WorldStream::new_with_asset_sets(
        protocol::WorldBootstrap {
            dimension: 0,
            local_player_runtime_id: 1,
            local_player_unique_id: 1,
            player_position: [0.0; 3],
            world_spawn_position: [0; 3],
            air_network_id: 0,
            block_network_ids_are_hashes: false,
        },
        Arc::new(assets::RuntimeAssets::diagnostic()),
        vanilla,
        [0.0; 3],
        None,
    );
    stream.set_pack_entities(Some((pack, vec![0])));
    stream
        .submit(
            1,
            protocol::WorldEvent::Actor(protocol::ActorEvent::Spawn(protocol::ActorSpawnEvent {
                dimension: 0,
                unique_id: 1,
                runtime_id: 1,
                kind: protocol::ActorKind::Player {
                    uuid: [0; 16],
                    username: "Offline".into(),
                },
                position: [0.0; 3],
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
    stream
}

#[test]
fn hud_uses_the_session_player_geometry_and_refreshes_between_catalogs() {
    let vanilla = player_catalog(8);
    let mut presentation =
        UiPresentationRuntime::new(super::super::super::tests::fixture_font()).unwrap();
    for size in [24, 32] {
        let stream = player_stream(Arc::clone(&vanilla), player_catalog(size));
        let rig = stream.actor_rig(1).expect("pack player rig");
        assert_eq!(rig.rig.0, assets::PACK_RIG_ID_BASE);
        assert!(rig.skin_geometry.is_none());
        presentation.capture_hud_player(Some(&stream), false);
        let live = &presentation.gui_models.live_player;
        assert!(
            !live.vertices.is_empty(),
            "server cube must replace the biped"
        );
        let width = live
            .vertices
            .iter()
            .map(|vertex| vertex.position[0])
            .fold(f32::NEG_INFINITY, f32::max)
            - live
                .vertices
                .iter()
                .map(|vertex| vertex.position[0])
                .fold(f32::INFINITY, f32::min);
        let expected = size as f32 / 16.0 / super::super::super::player_preview::PLAYER_MODEL_SCALE;
        assert!((width - expected).abs() < 1e-4, "{width} != {expected}");
    }
}
