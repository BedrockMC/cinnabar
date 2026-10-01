use crate::ui_runtime::{
    UiRuntime,
    inventory_router::{EquipmentRoute, EquipmentRouteResult},
};
use protocol::{
    ActorHandedness, ContainerIdentity, EquipmentEvent, InventoryContentEvent, InventoryEvent,
    InventorySlotEvent, NetworkItemStack, SlotIdentity,
};

fn assert_cube_scene(app: &bevy::prelude::App, expected: bool) {
    assert_eq!(
        app.world()
            .resource::<render::ViewmodelScene>()
            .is_opaque_cube(),
        expected
    );
}
fn container(window: i32) -> ContainerIdentity {
    ContainerIdentity {
        window_id: Some(window),
        slot_type: None,
        dynamic_id: None,
    }
}
fn equipment(actor: u64, stack: NetworkItemStack) -> EquipmentEvent {
    EquipmentEvent {
        actor_runtime_id: actor,
        stack,
        inventory_slot: 0,
        selected_slot: 0,
        window_id: 119,
        handedness: Some(ActorHandedness::Left),
    }
}
fn route(runtime: &mut UiRuntime, sequence: u64, event: EquipmentEvent) {
    let result = runtime
        .route_equipment(runtime.session_id(), sequence, event)
        .unwrap();
    if let EquipmentRouteResult::Routed(EquipmentRoute::LocalSelected {
        fifo_sequence,
        event,
    }) = result
    {
        runtime.retain_local_selected_equipment(fifo_sequence, event);
    }
}

#[test]
fn offhand_empty_provider_preserves_unknown_present_and_actual_authority_routes() {
    let mut runtime = UiRuntime::new(1);
    runtime.publish_local_runtime_id(1, 7).unwrap();
    assert_eq!(runtime.gameplay_hud().offhand_is_empty(), None);
    route(&mut runtime, 1, equipment(8, NetworkItemStack::empty()));
    assert_eq!(runtime.gameplay_hud().offhand_is_empty(), None);
    let mut present = NetworkItemStack::empty();
    present.count = 1;
    present.network_id = 1;
    route(&mut runtime, 2, equipment(7, present));
    assert_eq!(runtime.gameplay_hud().offhand_is_empty(), Some(false));
    assert!(runtime.gameplay_hud().offhand_stack().is_some());
    route(&mut runtime, 3, equipment(7, NetworkItemStack::empty()));
    assert_eq!(runtime.gameplay_hud().offhand_is_empty(), Some(true));
    assert!(runtime.gameplay_hud().offhand_stack().is_none());
    runtime.begin_session(2);
    assert_eq!(runtime.gameplay_hud().offhand_is_empty(), None);
    runtime
        .enqueue_inventory_event(
            2,
            1,
            InventoryEvent::Content(InventoryContentEvent {
                container: container(119),
                slots: vec![NetworkItemStack::empty()].into(),
                storage_item: NetworkItemStack::empty(),
            }),
        )
        .unwrap();
    runtime.drain_pending_inventory();
    assert_eq!(runtime.gameplay_hud().offhand_is_empty(), Some(true));
    runtime
        .enqueue_inventory_event(
            2,
            2,
            InventoryEvent::Slot(InventorySlotEvent {
                identity: SlotIdentity {
                    container: container(119),
                    slot: 999,
                },
                stack: {
                    let mut stack = NetworkItemStack::empty();
                    stack.network_id = 2;
                    stack.count = 1;
                    stack
                },
                storage_item: None,
            }),
        )
        .unwrap();
    runtime.drain_pending_inventory();
    assert_eq!(runtime.gameplay_hud().offhand_is_empty(), Some(true));
    runtime.begin_session(3);
    assert_eq!(runtime.gameplay_hud().offhand_is_empty(), None);
}

#[test]
fn cpu_hand_quad_is_retained_alongside_unchanged_held_items() {
    use crate::ui_runtime::presentation::{
        IconRef, UiPresentationRuntime, refresh_hud_frame, tests::fixture_font,
    };
    let mut presentation = UiPresentationRuntime::new(fixture_font()).unwrap();
    let pixels = vec![255; 64 * 64 * 4];
    presentation.set_player_preview_skin(Some(&pixels), Default::default());
    let mut runtime = UiRuntime::new(1);
    let settings = crate::camera::CameraSettingsAuthority::default();
    refresh_hud_frame(&mut runtime, &mut presentation, None, &settings, 0);
    let right = presentation.hud_frame().right_hand;
    assert!(right.is_some());
    let main_item = IconRef {
        page: 1,
        uv: [0, 0, 16, 16],
        glint: false,
    };
    let offhand = IconRef {
        page: 2,
        uv: [0, 0, 16, 16],
        glint: false,
    };
    presentation.hud_frame_mut().held_item_icon = Some(main_item);
    presentation.hud_frame_mut().offhand_viewmodel_icon = Some(offhand);
    assert_eq!(presentation.cpu_empty_hand_fallback(), right);
    assert_eq!(presentation.hud_frame().right_hand, right);
    assert_eq!(presentation.hud_frame().held_item_icon, Some(main_item));
    assert_eq!(
        presentation.hud_frame().offhand_viewmodel_icon,
        Some(offhand)
    );
    refresh_hud_frame(&mut runtime, &mut presentation, None, &settings, 1);
    assert_eq!(presentation.hud_frame().right_hand, right);
}

// The GPU first-person rig owns the hand: the HUD's CPU hand and held-item quads must not also
// draw in the screen corner.
#[test]
fn active_hand_rig_retires_the_cpu_hand_and_item_quads() {
    use crate::ui_runtime::presentation::{
        UiPresentationRuntime, refresh_hud_frame,
        tests::{fixture_font, fixture_hud},
    };
    let mut presentation = UiPresentationRuntime::with_hud(fixture_font(), fixture_hud()).unwrap();
    presentation.set_player_preview_skin(Some(&vec![255; 64 * 64 * 4]), Default::default());
    let mut runtime = UiRuntime::new(1);
    let settings = crate::camera::CameraSettingsAuthority::default();
    refresh_hud_frame(&mut runtime, &mut presentation, None, &settings, 0);
    let hand = presentation.hud_frame().right_hand.expect("hand carrier");
    let frame = presentation.hud_frame_mut();
    frame.first_person = true;
    frame.held_item_icon = Some(hand);
    let carriers = |presentation: &mut UiPresentationRuntime| {
        let input = presentation
            .build(&runtime, 0, [1280, 720], ui::DpiScale::new(1.).unwrap())
            .unwrap();
        let corner = [hand.uv[0], hand.uv[1]];
        let mut vertices = input
            .batches
            .iter()
            .filter(|batch| batch.texture_page == u32::from(hand.page))
            .flat_map(|batch| {
                let start = batch.first_index as usize;
                input.indices[start..start + batch.index_count as usize].to_vec()
            })
            .filter(|&index| input.vertices[index as usize].uv == corner)
            .collect::<Vec<_>>();
        vertices.sort_unstable();
        vertices.dedup();
        vertices.len()
    };
    assert_eq!(carriers(&mut presentation), 1, "a held item hides the arm");
    presentation.hud_frame_mut().held_item_icon = None;
    assert_eq!(
        carriers(&mut presentation),
        1,
        "the empty hand shows the arm"
    );
    presentation.hud_frame_mut().held_item_icon = Some(hand);
    presentation.hud_frame_mut().hand_rig_active = true;
    assert_eq!(carriers(&mut presentation), 0);
}

pub(super) struct FixturePack(std::path::PathBuf);
impl FixturePack {
    fn write(&self, path: &str, value: &[u8]) {
        let path = self.0.join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, value).unwrap();
    }
}
impl Drop for FixturePack {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
/// Compiles a small player rig with an arm and sleeve for hand-publication tests.
pub(super) fn hand_fixture() -> (
    FixturePack,
    render::ViewmodelGeometry,
    std::sync::Arc<assets::RuntimeEntityAssets>,
) {
    use assets::{RuntimeActorCatalog, RuntimeEntityAssets, encode_entity_blob};
    let root = std::env::temp_dir().join(format!(
        "neutral-hand-fixture-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir(&root).unwrap();
    let pack = FixturePack(root);
    for directory in ["animations", "animation_controllers"] {
        std::fs::create_dir(pack.0.join(directory)).unwrap();
    }
    pack.write("entity/player.json", br#"{"format_version":"1.10.0","minecraft:client_entity":{"description":{"identifier":"minecraft:player","geometry":{"default":"geometry.humanoid.custom"},"materials":{"default":"entity_alphatest"},"textures":{"default":"textures/entity/test"},"render_controllers":["controller.render.test"]}}}"#);
    let mut bones = Vec::new();
    for (name, parent, pivot) in [
        ("root", None, [0, 0, 0]),
        ("waist", Some("root"), [0, 12, 0]),
        ("body", Some("waist"), [0, 24, 0]),
        ("rightArm", Some("body"), [-5, 22, 0]),
        ("rightSleeve", Some("rightArm"), [-5, 22, 0]),
    ] {
        let mut bone = serde_json::json!({"name":name,"pivot":pivot});
        if let Some(parent) = parent {
            bone["parent"] = parent.into();
        }
        if matches!(name, "rightArm" | "rightSleeve") {
            bone["cubes"] = serde_json::json!([{"origin":[-8,12,-2],"size":[4,12,4],
                "uv":[40,if name == "rightArm" {16} else {32}], "inflate":if name == "rightArm" {0.0} else {0.25}}]);
        }
        bones.push(bone);
    }
    pack.write("models/entity/test.json", &serde_json::to_vec(&serde_json::json!({"format_version":"1.12.0",
        "minecraft:geometry":[{"description":{"identifier":"geometry.humanoid.custom","texture_width":64,"texture_height":64},"bones":bones}]})).unwrap());
    pack.write("render_controllers/test.json", br#"{"format_version":"1.8.0","render_controllers":{"controller.render.test":{"geometry":"Geometry.default","materials":[{"*":"Material.default"}],"textures":["Texture.default"]}}}"#);
    std::fs::create_dir_all(pack.0.join("textures/entity")).unwrap();
    image::RgbaImage::from_pixel(64, 64, image::Rgba([20, 40, 60, 255]))
        .save(pack.0.join("textures/entity/test.png"))
        .unwrap();
    let manifest = include_bytes!("../../../assets/vanilla-source.json");
    let compiled = asset_compiler::compile_entity_assets(&pack.0, manifest).unwrap();
    let bytes = encode_entity_blob(&compiled).unwrap();
    let entities = std::sync::Arc::new(RuntimeEntityAssets::decode(&bytes).unwrap());
    let actor = asset_compiler::compile_actor_assets(&pack.0, manifest).unwrap();
    let catalog = RuntimeActorCatalog::decode(&actor.bytes, &bytes).unwrap();
    let artwork = render::ActorArtworkPages::new(&catalog);
    let geometry = render::ViewmodelGeometry::from_runtime(&entities, &artwork).unwrap();
    (pack, geometry, entities)
}

#[test]
fn menu_input_leak_real_producer_to_hand_adapter_keeps_cpu_until_completion_and_clears_on_unknown_or_held()
 {
    use crate::{
        camera::FlyCamera,
        presentation::viewmodel::{HandAdapter, HandFallback, ViewmodelPublish},
        runtime::world::ClientWorld,
    };
    use bevy::{
        camera::{Camera, ComputedCameraValues, RenderTarget, RenderTargetInfo},
        ecs::system::RunSystemOnce,
        prelude::*,
    };
    use protocol::{
        ActorEvent, ActorKind, ActorSpawnEvent, PlayerListEntry, PlayerListUpdateEvent, PlayerSkin,
        StandardSkin, WorldBootstrap, WorldEvent,
    };
    use std::sync::Arc;
    let (_pack, geometry, entities) = hand_fixture();
    let assets = Arc::new(assets::RuntimeAssets::diagnostic());
    let mut stream = client_world::WorldStream::new_with_asset_sets(
        WorldBootstrap {
            local_player_unique_id: 1,
            dimension: 0,
            local_player_runtime_id: 1,
            player_position: [0., 64., 0.],
            world_spawn_position: [0, 64, 0],
            air_network_id: 0,
            block_network_ids_are_hashes: false,
        },
        assets.clone(),
        entities.clone(),
        [0., 64., 0.],
        None,
    );
    stream
        .submit(
            1,
            WorldEvent::Actor(ActorEvent::Spawn(ActorSpawnEvent {
                dimension: 0,
                unique_id: 1,
                runtime_id: 1,
                kind: ActorKind::Player {
                    uuid: [1; 16],
                    username: "test".into(),
                },
                position: [0., 64., 0.],
                velocity: [0.; 3],
                pitch: 0.,
                yaw: 0.,
                head_yaw: 0.,
                body_yaw: 0.,
                held_item: Default::default(),
                metadata: Arc::from([]),
                attributes: Arc::from([]),
                properties: Arc::from([]),
                links: Arc::from([]),
            })),
        )
        .unwrap();
    stream
        .submit(
            2,
            WorldEvent::Actor(ActorEvent::PlayerList(PlayerListUpdateEvent {
                entries: vec![PlayerListEntry::Add {
                    uuid: [1; 16],
                    unique_id: 1,
                    username: "test".into(),
                    verified: true,
                    skin: PlayerSkin::Standard(StandardSkin {
                        geometry: None,
                        cape: None,
                        width: 64,
                        height: 64,
                        rgba8: vec![255; 64 * 64 * 4].into(),
                    }),
                }]
                .into(),
            })),
        )
        .unwrap();
    let mut world = ClientWorld::new_with_entity_assets(assets, entities);
    world.stream = Some(stream);
    let mut runtime = UiRuntime::new(1);
    runtime.publish_local_runtime_id(1, 1).unwrap();
    runtime.retain_local_selected_equipment(
        1,
        EquipmentEvent {
            actor_runtime_id: 1,
            stack: NetworkItemStack::empty(),
            inventory_slot: 0,
            selected_slot: 0,
            window_id: 0,
            handedness: Some(ActorHandedness::Right),
        },
    );
    let mut app = App::new();
    app.insert_resource(runtime)
        .insert_resource(world)
        .insert_resource(geometry)
        .init_resource::<HandAdapter>()
        .init_resource::<render::ViewmodelScene>()
        .init_resource::<render::ViewmodelCompletionGate>();
    app.world_mut().spawn((
        FlyCamera::default(),
        Camera {
            computed: ComputedCameraValues {
                target_info: Some(RenderTargetInfo {
                    physical_size: UVec2::new(640, 480),
                    scale_factor: 1.0,
                }),
                ..Default::default()
            },
            ..Default::default()
        },
        RenderTarget::default(),
        Msaa::Off,
    ));
    let observe = |mut hand: ViewmodelPublish, runtime: Res<UiRuntime>, world: Res<ClientWorld>| {
        assert!(!hand.observe(&runtime, &world, true, false, [640, 480]));
    };
    app.world_mut().run_system_once(observe).unwrap();
    assert_eq!(
        app.world().resource::<HandAdapter>().stats.fallback,
        Some(HandFallback::ItemsUnknownOrHeld)
    );
    app.world_mut()
        .resource_mut::<UiRuntime>()
        .retain_local_selected_equipment(2, equipment(1, NetworkItemStack::empty()));
    app.world_mut().run_system_once(observe).unwrap();
    assert_eq!(
        app.world().resource::<HandAdapter>().stats.mode,
        Some(render::ViewmodelMode::EmptyHandNeutralStaticFallback)
    );
    assert_eq!(
        app.world().resource::<HandAdapter>().stats.skin_validations,
        1
    );
    let mut menu = crate::menu::MenuRuntime::new(false, 2, "Tester".into());
    menu.open_pause();
    app.insert_resource(menu);
    app.world_mut().run_system_once(observe).unwrap();
    assert!(app.world().resource::<HandAdapter>().stats.mode.is_some());
    app.world_mut()
        .resource_mut::<crate::menu::MenuRuntime>()
        .activate(crate::menu::MenuAction::PauseSettings);
    app.world_mut().run_system_once(observe).unwrap();
    assert!(app.world().resource::<HandAdapter>().stats.mode.is_none());
    app.world_mut()
        .resource_mut::<crate::menu::MenuRuntime>()
        .set_visible(false);
    app.world_mut()
        .run_system_once(
            |mut hand: ViewmodelPublish, runtime: Res<UiRuntime>, world: Res<ClientWorld>| {
                assert!(!hand.observe(&runtime, &world, false, false, [640, 480]));
            },
        )
        .unwrap();
    assert_eq!(
        app.world().resource::<HandAdapter>().stats.fallback,
        Some(HandFallback::Hidden)
    );
    app.world_mut()
        .run_system_once(
            |mut hand: ViewmodelPublish, runtime: Res<UiRuntime>, world: Res<ClientWorld>| {
                assert!(!hand.observe(&runtime, &world, true, true, [640, 480]));
            },
        )
        .unwrap();
    assert_eq!(app.world().resource::<HandAdapter>().stats.mode, None);
    app.world_mut()
        .run_system_once(
            |mut hand: ViewmodelPublish, runtime: Res<UiRuntime>, world: Res<ClientWorld>| {
                assert!(!hand.observe(&runtime, &world, true, false, [800, 480]));
            },
        )
        .unwrap();
    assert_eq!(
        app.world().resource::<HandAdapter>().stats.fallback,
        Some(HandFallback::View)
    );
    app.world_mut().run_system_once(observe).unwrap();
    assert_eq!(
        app.world().resource::<HandAdapter>().stats.skin_validations,
        1
    );
    let mut held = NetworkItemStack::empty();
    held.count = 1;
    held.network_id = 1;
    app.world_mut()
        .resource_mut::<UiRuntime>()
        .retain_local_selected_equipment(3, equipment(1, held));
    app.world_mut().run_system_once(observe).unwrap();
    assert_eq!(
        app.world().resource::<HandAdapter>().stats.fallback,
        Some(HandFallback::ItemsUnknownOrHeld)
    );
    app.world_mut().resource_mut::<UiRuntime>().begin_session(2);
    app.world_mut().run_system_once(observe).unwrap();
    assert_eq!(app.world().resource::<HandAdapter>().stats.mode, None);
    assert_eq!(
        app.world().resource::<HandAdapter>().stats.fallback,
        Some(HandFallback::Ownership)
    );
    // Both stacks may be explicitly empty but belong to a different retained
    // UI player identity. They cannot authorize the current stream's hand.
    {
        let mut runtime = app.world_mut().resource_mut::<UiRuntime>();
        runtime.publish_local_runtime_id(2, 99).unwrap();
        runtime.retain_local_selected_equipment(
            1,
            EquipmentEvent {
                actor_runtime_id: 99,
                stack: NetworkItemStack::empty(),
                inventory_slot: 0,
                selected_slot: 0,
                window_id: 0,
                handedness: Some(ActorHandedness::Right),
            },
        );
        runtime.retain_local_selected_equipment(2, equipment(99, NetworkItemStack::empty()));
    }
    app.world_mut().run_system_once(observe).unwrap();
    assert_eq!(
        app.world().resource::<HandAdapter>().stats.fallback,
        Some(HandFallback::Ownership)
    );
    assert_eq!(app.world().resource::<HandAdapter>().stats.mode, None);
    {
        let mut runtime = app.world_mut().resource_mut::<UiRuntime>();
        runtime.begin_session(3);
        runtime.publish_local_runtime_id(3, 1).unwrap();
        runtime.retain_local_selected_equipment(
            1,
            EquipmentEvent {
                actor_runtime_id: 1,
                stack: NetworkItemStack::empty(),
                inventory_slot: 0,
                selected_slot: 0,
                window_id: 0,
                handedness: Some(ActorHandedness::Right),
            },
        );
        runtime.retain_local_selected_equipment(2, equipment(1, NetworkItemStack::empty()));
    }
    app.world_mut().run_system_once(observe).unwrap();
    assert_eq!(
        app.world().resource::<HandAdapter>().stats.mode,
        Some(render::ViewmodelMode::EmptyHandNeutralStaticFallback)
    );
}

#[test]
fn ui_only_headless_hand_adapter_keeps_the_cpu_path_without_gpu_resources() {
    use crate::{presentation::viewmodel::ViewmodelPublish, runtime::world::ClientWorld};
    use bevy::{ecs::system::RunSystemOnce, prelude::*};
    let (_pack, _geometry, entities) = hand_fixture();
    let mut app = App::new();
    app.insert_resource(UiRuntime::new(1))
        .insert_resource(ClientWorld::new_with_entity_assets(
            std::sync::Arc::new(assets::RuntimeAssets::diagnostic()),
            entities,
        ));
    app.world_mut()
        .run_system_once(
            |mut hand: ViewmodelPublish, runtime: Res<UiRuntime>, world: Res<ClientWorld>| {
                assert!(!hand.observe(&runtime, &world, true, false, [640, 480]));
                hand.clear();
            },
        )
        .unwrap();
}

fn cube_world_assets(
    entities: &assets::RuntimeEntityAssets,
) -> std::sync::Arc<assets::RuntimeAssets> {
    use assets::*;
    use sha2::{Digest, Sha256};
    let count = entities.block_visual_count() as usize;
    let mut visuals = vec![
        BlockVisual {
            faces: [1; 6],
            flags: BlockFlags::CUBE_GEOMETRY | BlockFlags::OCCLUDES_FULL_FACE,
            kind: VisualKind::Cube,
            support: VisualSupport::Exact,
            contributor_role: ContributorRole::Primary,
            model_template: NO_MODEL_TEMPLATE,
            animation: NO_ANIMATION,
            variant: 0,
        };
        count
    ];
    visuals[0] = BlockVisual::diagnostic(BlockFlags::empty(), ContributorRole::Primary);
    let source = CompiledAssets {
        visuals: visuals.into(),
        light_properties: vec![LightProperties::default(); count].into(),
        hashed: Box::new([]),
        materials: vec![
            Material {
                texture: TextureRef::new(0, 0).unwrap(),
                flags: 0,
                animation: NO_ANIMATION
            };
            2
        ]
        .into(),
        model_templates: Box::new([]),
        model_quads: Box::new([]),
        animations: Box::new([]),
        animation_frames: Box::new([]),
        texture_pages: vec![TexturePage::new(TextureArray {
            layers: 1,
            mips: [16, 8, 4, 2, 1]
                .into_iter()
                .map(|size| TextureMip {
                    size,
                    rgba8: vec![255; size as usize * size as usize * 4].into(),
                })
                .collect::<Vec<_>>()
                .into(),
        })]
        .into(),
        biomes: CompiledBiomeAssets::diagnostic(),
        provenance: BlobProvenance {
            source_manifest_sha256: entities.source_manifest_sha256(),
            block_registry_sha256: Sha256::digest(
                crate::asset_startup::pinned_block_registry_bytes(),
            )
            .into(),
            light_registry_sha256: [3; 32],
            biome_registry_sha256: [4; 32],
        },
    };
    std::sync::Arc::new(RuntimeAssets::decode(&assets::encode_blob(&source).unwrap()).unwrap())
}

#[test]
fn real_selected_block_provider_and_rotated_ui_publisher_bind_cube_and_clear_rejection() {
    use crate::ui_runtime::presentation::{
        UiPresentationRuntime, refresh_hud_frame,
        tests::{fixture_font, fixture_hud},
    };
    use crate::{
        camera::FlyCamera,
        presentation::viewmodel::{HandAdapter, HandFallback, ViewmodelPublish},
        runtime::world::ClientWorld,
    };
    use bevy::{
        camera::{Camera, ComputedCameraValues, RenderTarget, RenderTargetInfo},
        ecs::system::RunSystemOnce,
        prelude::*,
    };
    use protocol::WorldBootstrap;
    use std::sync::Arc;
    let (pack, _geometry, _) = hand_fixture();
    // Match the decoded carrier's unsupported player-controller route: retain
    // the player symbol, authored geometry and item routes, but no resolved rig.
    let mut compiled = asset_compiler::compile_entity_assets(
        &pack.0,
        include_bytes!("../../../assets/vanilla-source.json"),
    )
    .unwrap();
    compiled.rig_bindings = Box::new([]);
    compiled.rig_geometries = Box::new([]);
    compiled.rig_animations = Box::new([]);
    compiled.rig_controllers = Box::new([]);
    compiled.render = Default::default();
    let entities = Arc::new(
        assets::RuntimeEntityAssets::decode(&assets::encode_entity_blob(&compiled).unwrap())
            .unwrap(),
    );
    assert!(
        !entities
            .geometry_candidates("geometry.humanoid.custom")
            .is_empty()
    );
    assert!(entities.rig_bindings().is_empty());
    assert!(entities.rig_geometries().is_empty());
    let assets = cube_world_assets(&entities);
    let bootstrap = WorldBootstrap {
        local_player_unique_id: 1,
        dimension: 0,
        local_player_runtime_id: 1,
        player_position: [0., 64., 0.],
        world_spawn_position: [0, 64, 0],
        air_network_id: 0,
        block_network_ids_are_hashes: false,
    };
    let stream = client_world::WorldStream::new_with_asset_sets(
        bootstrap,
        assets.clone(),
        entities.clone(),
        [0., 64., 0.],
        None,
    );
    assert!(stream.actor(1).is_none());
    assert!(stream.actor_rig(1).is_none());
    let mut world = ClientWorld::new_with_entity_assets(assets, entities);
    world.stream = Some(stream);
    let mut runtime = UiRuntime::new(1);
    runtime.publish_local_runtime_id(1, 1).unwrap();
    let item = protocol::vanilla_item_registry()
        .iter()
        .find(|item| item.identifier.as_ref() == "minecraft:dirt")
        .unwrap()
        .network_id;
    let mut stack = NetworkItemStack::empty();
    stack.network_id = item;
    stack.count = 1;
    stack.stack_network_id = 2;
    stack.extra_data = Arc::from([0; 10]);
    stack.nbt_digest = {
        use sha2::Digest;
        sha2::Sha256::digest(&stack.extra_data).into()
    };
    let session = protocol::BedrockSession { shield_item_id: 0 };
    let wire = protocol::encode(
        &protocol::select_hotbar_slot_packet(1, 0, &stack).unwrap(),
        &session,
    )
    .unwrap();
    let decoded = protocol::decode_batch(wire, &session)
        .unwrap()
        .pop()
        .unwrap();
    let Some(protocol::WorldEvent::Equipment(decoded)) =
        protocol::into_world_event(decoded, 0).unwrap()
    else {
        panic!("expected selected equipment");
    };
    assert_eq!(decoded.stack, stack);
    let held = EquipmentEvent {
        actor_runtime_id: 1,
        stack: decoded.stack,
        inventory_slot: 0,
        selected_slot: 0,
        window_id: 0,
        handedness: Some(ActorHandedness::Right),
    };
    runtime.retain_local_selected_equipment(1, held.clone());
    runtime.retain_local_selected_equipment(2, equipment(1, NetworkItemStack::empty()));
    let mut presentation = UiPresentationRuntime::with_hud(fixture_font(), fixture_hud()).unwrap();
    presentation.set_player_preview_skin(Some(&vec![255; 64 * 64 * 4]), Default::default());
    refresh_hud_frame(
        &mut runtime,
        &mut presentation,
        world.stream.as_ref(),
        &Default::default(),
        0,
    );
    // Block-routed items are absent from the sprite-only icon catalog. The
    // real right-hand carrier remains visible until current cube completion.
    assert!(presentation.hud_frame().held_item_icon.is_none());
    assert!(presentation.cpu_empty_hand_fallback().is_some());
    presentation.hud_frame_mut().viewmodel_pitch_degrees = 30.;
    let input = presentation
        .build(&runtime, 0, [640, 480], ui::DpiScale::new(1.).unwrap())
        .unwrap();
    let empty = presentation.cpu_empty_hand_fallback();
    let mut app = App::new();
    let mut movement = crate::movement::MovementTicker::default();
    let mut physics = crate::movement::LocalPhysicsController::default();
    crate::movement::reset_start_game_prediction(&mut movement, &mut physics, 1, [0., 64., 0.]);
    movement.set_source(crate::movement::MovementSource::Physics);
    let mut avatar = crate::local_player::LocalAvatarPresentation::default();
    let mut view = crate::local_player::LocalViewPose::default();
    let mut settings = crate::camera::CameraSettingsAuthority::default();
    crate::local_player::reset_local_player_session(
        1,
        1,
        [0., 64., 0.],
        &mut settings,
        &mut view,
        &mut avatar,
    );
    let mut visibility = crate::local_player::LocalAvatarVisibilityCarrier::default();
    avatar.publish_view_visibility(
        semantic_input::PerspectiveMode::FirstPerson,
        Vec3::new(0., 64., 0.),
        Quat::IDENTITY,
        &mut visibility,
    );
    app.insert_resource(runtime)
        .insert_resource(world)
        .insert_resource(movement)
        .insert_resource(visibility)
        .init_resource::<HandAdapter>()
        .init_resource::<render::ViewmodelScene>()
        .init_resource::<render::ViewmodelCompletionGate>();
    assert!(!app.world().contains_resource::<render::ViewmodelGeometry>());
    app.world_mut().spawn((
        FlyCamera::default(),
        Camera {
            computed: ComputedCameraValues {
                target_info: Some(RenderTargetInfo {
                    physical_size: UVec2::new(640, 480),
                    scale_factor: 1.,
                }),
                ..Default::default()
            },
            ..Default::default()
        },
        RenderTarget::default(),
        Msaa::Off,
    ));
    let observed_input = input.clone();
    app.world_mut()
        .run_system_once(
            move |mut hand: ViewmodelPublish, runtime: Res<UiRuntime>, world: Res<ClientWorld>| {
                assert!(!hand.observe(&runtime, &world, true, false, [640, 480]));
                hand.bind_cpu_fallback(&observed_input, empty, None);
                let (reason, values) =
                    hand.diagnostic_snapshot(&runtime, &world, true, false, [640, 480], false);
                assert_eq!(reason, 0);
                assert_eq!(values[4], 0);
                assert_eq!(values[6], 2);
                assert_eq!(values[8], i128::from(item));
                assert_eq!(values[9], 1);
                assert_eq!(values[14], 2);
                assert_eq!(values[21], 1);
                assert_eq!(values[23], 0);
                assert_eq!(values[24], 0);
            },
        )
        .unwrap();
    assert_cube_scene(&app, true);
    assert_eq!(
        app.world().resource::<HandAdapter>().stats.mode,
        Some(render::ViewmodelMode::OpaqueCubeNeutralStaticFallback)
    );
    // Exercise the real committed-control reconciliation, not a fabricated
    // actor spawn counter. The free-camera correction branch anchors without
    // requiring loaded collision chunks; cube admission resumes only afterward.
    let mut clock = crate::environment::WorldClock::default();
    let mut weather = crate::environment::WeatherState::default();
    crate::environment::bind_session_generation(&mut clock, &mut weather, 1);
    let breg = crate::asset_startup::pinned_block_registry_bytes();
    let records = assets::read_registry_for_protocol(breg, 2193).unwrap();
    let collisions = crate::movement::PhysicsCollisionRegistries::from_assets(
        breg,
        &records,
        include_bytes!("../../../crates/assets/data/block-physics-v2193.bin"),
        2193,
    )
    .unwrap();
    app.insert_resource(clock)
        .insert_resource(weather)
        .insert_resource(collisions)
        .insert_resource(physics)
        .insert_resource(crate::acceptance::AcceptanceRun::new(
            Some(900),
            None,
            false,
            false,
        ))
        .insert_resource(crate::acceptance::model_witness::ModelWitnessFileSource::new(None))
        .init_resource::<crate::movement::LocalMovementEffectTimeline>()
        .init_resource::<crate::movement::LocalMovementSpeedAuthority>()
        .init_resource::<Time<bevy::time::Real>>()
        .init_resource::<render::ChunkUploadBudget>()
        .init_resource::<crate::camera::CameraSettingsAuthority>()
        .init_resource::<crate::local_player::LocalViewPose>()
        .init_resource::<crate::local_player::LocalPlayerFrameCarrier>()
        .init_resource::<crate::local_player::InteractionOriginSnapshot>()
        .init_resource::<crate::runtime::phase3_evidence::Phase3EvidenceEmitter>()
        .init_resource::<crate::runtime::world::WorldStreamFramePoll>()
        .init_resource::<crate::server_camera::ServerCameraInstructions>()
        .add_message::<crate::runtime::audio::SequencedAudioEvent>();
    let controls = [
        protocol::WorldEvent::Respawn(protocol::RespawnEvent {
            position: [1., 64., 0.],
            state: 1,
            runtime_entity_id: 1,
        }),
        protocol::WorldEvent::MovePlayer(protocol::MovePlayerEvent {
            runtime_id: 1,
            position: [2., 64., 0.],
            teleported: true,
            ..Default::default()
        }),
        protocol::WorldEvent::ChangeDimension(protocol::ChangeDimensionEvent {
            dimension: 1,
            position: [3., 64., 0.],
        }),
        protocol::WorldEvent::ChangeDimension(protocol::ChangeDimensionEvent {
            dimension: 0,
            position: [4., 64., 0.],
        }),
    ];
    for (index, event) in controls.into_iter().enumerate() {
        app.world_mut()
            .resource_mut::<crate::movement::MovementTicker>()
            .set_source(crate::movement::MovementSource::FreeCamera);
        let before = app
            .world()
            .resource::<crate::movement::MovementTicker>()
            .interaction_authority_identity();
        app.world_mut()
            .resource_mut::<ClientWorld>()
            .stream
            .as_mut()
            .unwrap()
            .submit(index as u64 + 1, event)
            .unwrap();
        app.world_mut()
            .run_system_once(crate::runtime::world::reconcile_world_stream_before_physics)
            .unwrap();
        let after = app
            .world()
            .resource::<crate::movement::MovementTicker>()
            .interaction_authority_identity();
        assert_eq!(before.0, after.0);
        assert!(after.1 > before.1);
        app.world_mut()
            .run_system_once(
                |mut hand: ViewmodelPublish, runtime: Res<UiRuntime>, world: Res<ClientWorld>| {
                    assert!(!hand.observe(&runtime, &world, true, false, [640, 480]));
                },
            )
            .unwrap();
        assert_cube_scene(&app, false);
        app.world_mut()
            .resource_mut::<crate::movement::MovementTicker>()
            .set_source(crate::movement::MovementSource::Physics);
        let input = input.clone();
        app.world_mut()
            .run_system_once(
                move |mut hand: ViewmodelPublish,
                      runtime: Res<UiRuntime>,
                      world: Res<ClientWorld>| {
                    assert!(!hand.observe(&runtime, &world, true, false, [640, 480]));
                    hand.bind_cpu_fallback(&input, empty, None);
                },
            )
            .unwrap();
        assert_cube_scene(&app, true);
        assert!(app.world().resource::<ClientWorld>().fatal_error.is_none());
    }
    assert!(
        input
            .vertices
            .iter()
            .any(|vertex| vertex.position[0] > 640.)
    );
    let mut bad = input.clone();
    bad.viewport_size[0] += 1;
    app.world_mut()
        .run_system_once(move |mut hand: ViewmodelPublish| {
            hand.bind_cpu_fallback(&bad, empty, None)
        })
        .unwrap();
    assert_cube_scene(&app, false);
    let mut mismatched = held.clone();
    mismatched.stack.block_runtime_id = i32::MAX;
    app.world_mut()
        .resource_mut::<UiRuntime>()
        .retain_local_selected_equipment(3, mismatched);
    app.world_mut()
        .run_system_once(
            |mut hand: ViewmodelPublish, runtime: Res<UiRuntime>, world: Res<ClientWorld>| {
                assert!(!hand.observe(&runtime, &world, true, false, [640, 480]));
            },
        )
        .unwrap();
    assert_cube_scene(&app, false);
    assert_eq!(
        app.world().resource::<HandAdapter>().stats.fallback,
        Some(HandFallback::ItemsUnknownOrHeld)
    );
    app.world_mut()
        .resource_mut::<UiRuntime>()
        .retain_local_selected_equipment(4, held.clone());
    let recovered_input = input.clone();
    app.world_mut()
        .run_system_once(
            move |mut hand: ViewmodelPublish, runtime: Res<UiRuntime>, world: Res<ClientWorld>| {
                assert!(!hand.observe(&runtime, &world, true, false, [640, 480]));
                hand.bind_cpu_fallback(&recovered_input, empty, None);
            },
        )
        .unwrap();
    assert_cube_scene(&app, true);
    app.world_mut()
        .resource_mut::<UiRuntime>()
        .retain_local_selected_equipment(
            5,
            EquipmentEvent {
                stack: NetworkItemStack::empty(),
                ..held.clone()
            },
        );
    app.world_mut()
        .run_system_once(
            |mut hand: ViewmodelPublish, runtime: Res<UiRuntime>, world: Res<ClientWorld>| {
                assert!(!hand.observe(&runtime, &world, true, false, [640, 480]));
            },
        )
        .unwrap();
    assert_eq!(
        app.world().resource::<HandAdapter>().stats.fallback,
        Some(HandFallback::Ownership)
    );
    assert!(app.world().resource::<HandAdapter>().stats.mode.is_none());
    assert_cube_scene(&app, false);
    app.world_mut()
        .resource_mut::<UiRuntime>()
        .retain_local_selected_equipment(6, held.clone());
    let resumed_input = input.clone();
    app.world_mut()
        .run_system_once(
            move |mut hand: ViewmodelPublish, runtime: Res<UiRuntime>, world: Res<ClientWorld>| {
                assert!(!hand.observe(&runtime, &world, true, false, [640, 480]));
                hand.bind_cpu_fallback(&resumed_input, empty, None);
            },
        )
        .unwrap();
    assert_cube_scene(&app, true);
    app.world_mut().resource_mut::<UiRuntime>().begin_session(2);
    app.world_mut()
        .run_system_once(
            |mut hand: ViewmodelPublish, runtime: Res<UiRuntime>, world: Res<ClientWorld>| {
                assert!(!hand.observe(&runtime, &world, true, false, [640, 480]));
            },
        )
        .unwrap();
    assert_cube_scene(&app, false);
    assert_eq!(
        app.world().resource::<HandAdapter>().stats.fallback,
        Some(HandFallback::Ownership)
    );
    {
        let mut runtime = app.world_mut().resource_mut::<UiRuntime>();
        runtime.publish_local_runtime_id(2, 1).unwrap();
        runtime.retain_local_selected_equipment(1, held);
        runtime.retain_local_selected_equipment(2, equipment(1, NetworkItemStack::empty()));
    }
    app.world_mut()
        .resource_mut::<crate::movement::MovementTicker>()
        .reset(2, 0, [0., 64., 0.]);
    let mut avatar = crate::local_player::LocalAvatarPresentation::default();
    avatar.begin_session(2, 1);
    avatar.publish_view_visibility(
        semantic_input::PerspectiveMode::FirstPerson,
        Vec3::new(0., 64., 0.),
        Quat::IDENTITY,
        &mut app
            .world_mut()
            .resource_mut::<crate::local_player::LocalAvatarVisibilityCarrier>(),
    );
    for fresh_stream in [false, true] {
        if fresh_stream {
            let mut world = app.world_mut().resource_mut::<ClientWorld>();
            let fresh = client_world::WorldStream::new_with_asset_sets(
                bootstrap,
                world.runtime_assets.clone(),
                world.entity_assets.clone().unwrap(),
                bootstrap.player_position,
                None,
            );
            assert!(fresh.actor_session_id() > world.stream.as_ref().unwrap().actor_session_id());
            assert!(fresh.actor(1).is_none());
            world.stream = Some(fresh);
        }
        let current_input = input.clone();
        app.world_mut()
            .run_system_once(
                move |mut hand: ViewmodelPublish,
                      runtime: Res<UiRuntime>,
                      world: Res<ClientWorld>| {
                    assert!(!hand.observe(&runtime, &world, true, false, [640, 480]));
                    hand.bind_cpu_fallback(&current_input, empty, None);
                },
            )
            .unwrap();
        assert_cube_scene(&app, fresh_stream);
    }
    // Retiring genuine local authority withholds the cube; no remote spawn was
    // ever installed, and a visibility snapshot alone cannot grant admission.
    app.world_mut()
        .resource_mut::<crate::movement::MovementTicker>()
        .deactivate();
    app.world_mut()
        .run_system_once(
            |mut hand: ViewmodelPublish, runtime: Res<UiRuntime>, world: Res<ClientWorld>| {
                assert!(world.stream.as_ref().unwrap().actor(1).is_none());
                assert!(!hand.observe(&runtime, &world, true, false, [640, 480]));
                let (reason, values) =
                    hand.diagnostic_snapshot(&runtime, &world, true, false, [640, 480], false);
                assert_eq!(reason, 3);
                assert_eq!(values[4], 0);
                assert_eq!(values[21], 0);
            },
        )
        .unwrap();
    assert_eq!(
        app.world().resource::<HandAdapter>().stats.fallback,
        Some(HandFallback::Ownership)
    );
    assert_cube_scene(&app, false);
}
