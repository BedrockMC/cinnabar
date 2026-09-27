//! Actual inventory ingress, world reconcile and committed authority drain witnesses.
use crate::{
    acceptance::{AcceptanceRun, model_witness::ModelWitnessFileSource},
    camera::CameraSettingsAuthority,
    environment::{WeatherState, WorldClock, bind_session_generation},
    local_player::{InteractionOriginSnapshot, LocalPlayerFrameCarrier, LocalViewPose},
    movement::{
        LocalMovementEffectTimeline, LocalMovementSpeedAuthority, LocalPhysicsController,
        MovementTicker, PhysicsCollisionRegistries,
    },
    runtime::{
        audio::SequencedAudioEvent,
        network::{
            publish_bootstrap_inventory, route_inventory_ingress, route_item_registry_ingress,
            session::SequencedWorldEvent,
        },
        phase3_evidence::Phase3EvidenceEmitter,
        world::{
            ClientWorld, WorldStreamFramePoll, drain_committed_ui_before_authority,
            reconcile_world_stream_before_physics,
        },
    },
    server_camera::ServerCameraInstructions,
    ui_runtime::{
        CraftingPreview, UiRuntime, drain_inventory_authority,
        inventory_ledger::PlayerInventorySlot,
    },
};
use bevy::{prelude::*, time::Real};
use protocol::{
    ContainerIdentity, InventoryAuthority, InventoryEvent, InventorySlotEvent, ItemRegistryEvent,
    NetworkItemStack, SlotIdentity, WorldBootstrap, WorldEvent,
};

fn app() -> App {
    let mut clock = WorldClock::default();
    let mut weather = WeatherState::default();
    bind_session_generation(&mut clock, &mut weather, 1);
    let breg = include_bytes!("../../../crates/assets/data/block-registry-v2168.bin");
    let preg = include_bytes!("../../../crates/assets/data/block-physics-v2168.bin");
    let records = assets::read_registry_for_protocol(breg, 2168).unwrap();
    let collisions = PhysicsCollisionRegistries::from_assets(breg, &records, preg, 2168).unwrap();
    let mut runtime = UiRuntime::new(1);
    assert!(publish_bootstrap_inventory(
        &mut runtime,
        Some(ItemRegistryEvent {
            entries: protocol::vanilla_item_registry(),
        }),
        InventoryEvent::Authority(InventoryAuthority::Server)
    ));
    let stream = client_world::WorldStream::new(WorldBootstrap {
        dimension: 0,
        local_player_runtime_id: 42,
        local_player_unique_id: 1,
        player_position: [0.0, 70.0, 0.0],
        world_spawn_position: [0, 70, 0],
        air_network_id: protocol::SEQUENTIAL_AIR_NETWORK_ID,
        block_network_ids_are_hashes: false,
    });
    let mut app = App::new();
    app.insert_resource(ClientWorld {
        stream: Some(stream),
        ..ClientWorld::default()
    })
    .insert_resource(clock)
    .insert_resource(weather)
    .insert_resource(collisions)
    .insert_resource(runtime)
    .insert_resource(AcceptanceRun::new(Some(900), None, false, false))
    .insert_resource(ModelWitnessFileSource::new(None))
    .init_resource::<MovementTicker>()
    .init_resource::<LocalPhysicsController>()
    .init_resource::<LocalMovementEffectTimeline>()
    .init_resource::<LocalMovementSpeedAuthority>()
    .init_resource::<Time<Real>>()
    .init_resource::<render::ChunkUploadBudget>()
    .init_resource::<CameraSettingsAuthority>()
    .init_resource::<LocalViewPose>()
    .init_resource::<LocalPlayerFrameCarrier>()
    .init_resource::<InteractionOriginSnapshot>()
    .init_resource::<Phase3EvidenceEmitter>()
    .init_resource::<WorldStreamFramePoll>()
    .init_resource::<ServerCameraInstructions>()
    .add_message::<SequencedAudioEvent>()
    .add_systems(
        Update,
        (
            reconcile_world_stream_before_physics,
            drain_committed_ui_before_authority,
            drain_inventory_authority,
        )
            .chain(),
    );
    app
}

fn ingress(app: &mut App, sequence: u64, event: InventoryEvent) {
    let mut runtime = app.world_mut().resource_mut::<UiRuntime>();
    route_inventory_ingress(
        &mut runtime,
        SequencedWorldEvent {
            session_generation: 1,
            sequence,
            event: WorldEvent::Inventory(event),
        },
    )
    .unwrap();
    drop(runtime);
    app.world_mut()
        .resource_mut::<ClientWorld>()
        .stream
        .as_mut()
        .unwrap()
        .commit(sequence)
        .unwrap();
}

fn empty_slot(name: u8, slot: u16) -> InventoryEvent {
    InventoryEvent::Slot(InventorySlotEvent {
        identity: SlotIdentity {
            container: ContainerIdentity {
                window_id: Some(if name == 13 { 124 } else { 0 }),
                slot_type: Some(name),
                dynamic_id: None,
            },
            slot,
        },
        stack: NetworkItemStack::empty(),
        storage_item: None,
    })
}

fn clear_recipes() -> InventoryEvent {
    // Exact supported-empty body already covered by protocol admission witnesses.
    let mut body = [0; 12];
    body[11] = 1;
    InventoryEvent::Recipes(protocol::decode_recipe_update(&body).unwrap())
}

fn named_registry(name: &str) -> ItemRegistryEvent {
    ItemRegistryEvent {
        entries: [(6, name), (7, "minecraft:oak_planks")]
            .into_iter()
            .map(|(network_id, identifier)| protocol::ItemRegistryEntry {
                identifier: identifier.into(),
                network_id,
                component_based: false,
                version: protocol::ItemRegistryVersion::None,
                component_digest: [0; 32],
                negotiated_max_stack_size: Some(64),
                canonical_empty_component_data: true,
            })
            .collect(),
    }
}

fn registry_ingress(app: &mut App, sequence: u64, registry: ItemRegistryEvent) {
    route_item_registry_ingress(
        &mut app.world_mut().resource_mut::<UiRuntime>(),
        &SequencedWorldEvent {
            session_generation: 1,
            sequence,
            event: WorldEvent::ItemActor(protocol::ItemActorEvent::Registry(registry)),
        },
    )
    .unwrap();
    app.world_mut()
        .resource_mut::<ClientWorld>()
        .stream
        .as_mut()
        .unwrap()
        .commit(sequence)
        .unwrap();
}

#[test]
fn committed_registry_position_controls_cell_binding_even_when_ordinary_registry_runs_ahead() {
    let mut app = app();
    registry_ingress(&mut app, 1, named_registry("minecraft:oak_log"));
    let fixture =
        include_bytes!("../../../crates/protocol/fixtures/crafting_data_manual_named_1x1.bin");
    // Pinned raw batch: one107-byte packet with a two-byte game header.
    assert_eq!(&fixture[..4], &[0xfe, 0x6b, 0xb4, 0x48]);
    ingress(
        &mut app,
        2,
        InventoryEvent::Recipes(protocol::decode_recipe_update(&fixture[4..]).unwrap()),
    );
    let present = || {
        let mut event = empty_slot(13, 28);
        let InventoryEvent::Slot(update) = &mut event else {
            unreachable!()
        };
        update.stack = NetworkItemStack {
            network_id: 6,
            stack_network_id: 101,
            count: 1,
            ..NetworkItemStack::empty()
        };
        event
    };
    ingress(&mut app, 3, present());
    for (sequence, slot) in [(4, 29), (5, 30), (6, 31)] {
        ingress(&mut app, sequence, empty_slot(13, slot));
    }
    ingress(&mut app, 7, empty_slot(59, 0));
    // RegistryB is already applied by the unchanged ordinary drain, but the
    // missing predecessor8 keeps crafting bound to the committed RegistryA.
    registry_ingress(&mut app, 9, named_registry("minecraft:birch_log"));
    app.update();
    let runtime = app.world().resource::<UiRuntime>();
    assert_eq!(
        runtime
            .inventory_ledger()
            .negotiated_item_entry(6)
            .unwrap()
            .identifier
            .as_ref(),
        "minecraft:birch_log"
    );
    assert_eq!(
        runtime.crafting_preview(),
        Some(CraftingPreview::Unique {
            identifier: "minecraft:oak_planks",
            count: 4,
            metadata: 0,
            block_runtime_id: 0,
        })
    );
    let old = runtime.clone();
    app.world_mut()
        .resource_mut::<ClientWorld>()
        .stream
        .as_mut()
        .unwrap()
        .commit(8)
        .unwrap();
    app.update();
    assert!(
        app.world()
            .resource::<UiRuntime>()
            .crafting_preview()
            .is_none(),
        "registry replacement must not rebind an older numeric cell"
    );
    assert!(
        matches!(old.crafting_preview(), Some(CraftingPreview::Unique { .. })),
        "an older immutable runtime keeps its original preview and credited owners"
    );
    ingress(&mut app, 10, present());
    app.update();
    assert_eq!(
        app.world().resource::<UiRuntime>().crafting_preview(),
        Some(CraftingPreview::NoMatch)
    );
    ingress(&mut app, 11, clear_recipes());
    app.update();
    assert!(
        matches!(old.crafting_preview(), Some(CraftingPreview::Unique { .. })),
        "new catalog updates cannot mutate an older cloned preview"
    );
    assert!(app.world().resource::<ClientWorld>().fatal_error.is_none());
}

fn complete_empty_grid(app: &mut App, first: u64) {
    for index in 0..4 {
        ingress(app, first + index, empty_slot(13, 28 + index as u16));
    }
    ingress(app, first + 4, empty_slot(59, 0));
}

#[test]
fn missing_world_predecessor_withholds_crafting_but_not_ordinary_inventory_then_releases_fifo() {
    let mut app = app();
    ingress(&mut app, 2, clear_recipes());
    complete_empty_grid(&mut app, 3);
    ingress(&mut app, 8, empty_slot(12, 28));
    app.update();
    let runtime = app.world().resource::<UiRuntime>();
    assert!(matches!(
        runtime.inventory_ledger().slot_state(28),
        Some(PlayerInventorySlot::Empty)
    ));
    assert!(runtime.crafting_preview().is_none());
    app.world_mut()
        .resource_mut::<ClientWorld>()
        .stream
        .as_mut()
        .unwrap()
        .commit(1)
        .unwrap();
    app.update();
    assert_eq!(
        app.world().resource::<UiRuntime>().crafting_preview(),
        Some(CraftingPreview::NoMatch)
    );
    assert!(app.world().resource::<ClientWorld>().fatal_error.is_none());
}

#[test]
fn slot_only_projection_overflow_does_not_disconnect_or_discard_healthy_bootstrap_domains() {
    let mut app = app();
    ingress(&mut app, 1, clear_recipes());
    app.update();
    // Keep predecessor2 absent while ordinary drain remains destructive each frame.
    for sequence in 3..=66 {
        ingress(&mut app, sequence, empty_slot(13, 28));
        app.update();
    }
    // The existing world cap is64. Release its predecessor before admitting the
    // next event, but before the craft observer drains its retained64 records.
    app.world_mut()
        .resource_mut::<ClientWorld>()
        .stream
        .as_mut()
        .unwrap()
        .commit(2)
        .unwrap();
    ingress(&mut app, 67, empty_slot(13, 28));
    app.update();
    ingress(&mut app, 68, empty_slot(12, 31));
    app.update();
    assert!(matches!(
        app.world()
            .resource::<UiRuntime>()
            .inventory_ledger()
            .slot_state(31),
        Some(PlayerInventorySlot::Empty)
    ));
    assert!(
        app.world()
            .resource::<UiRuntime>()
            .crafting_preview()
            .is_none()
    );
    assert!(app.world().resource::<ClientWorld>().fatal_error.is_none());
    complete_empty_grid(&mut app, 69);
    app.update();
    assert_eq!(
        app.world().resource::<UiRuntime>().crafting_preview(),
        Some(CraftingPreview::NoMatch)
    );
}

#[test]
fn immediate_client_loss_blocks_queued_older_server_when_only_the_older_prefix_commits() {
    let mut app = app();
    ingress(&mut app, 1, clear_recipes());
    complete_empty_grid(&mut app, 2);
    app.update();
    assert!(
        app.world()
            .resource::<UiRuntime>()
            .crafting_preview()
            .is_some()
    );
    ingress(
        &mut app,
        7,
        InventoryEvent::Authority(InventoryAuthority::Server),
    );
    // Sequence8 is absent;9 is admitted but cannot yet be committed.
    ingress(
        &mut app,
        9,
        InventoryEvent::Authority(InventoryAuthority::Client),
    );
    assert!(
        app.world()
            .resource::<UiRuntime>()
            .crafting_preview()
            .is_none()
    );
    app.update();
    assert!(
        app.world()
            .resource::<UiRuntime>()
            .crafting_preview()
            .is_none()
    );
    assert_eq!(
        app.world()
            .resource::<ClientWorld>()
            .stream
            .as_ref()
            .unwrap()
            .inventory_committed_through(),
        Some(7)
    );
    app.world_mut()
        .resource_mut::<ClientWorld>()
        .stream
        .as_mut()
        .unwrap()
        .commit(8)
        .unwrap();
    ingress(
        &mut app,
        10,
        InventoryEvent::Authority(InventoryAuthority::Server),
    );
    complete_empty_grid(&mut app, 11);
    app.update();
    assert_eq!(
        app.world().resource::<UiRuntime>().crafting_preview(),
        Some(CraftingPreview::NoMatch)
    );
}

#[test]
fn actual_dimension_boundaries_drop_old_cells_and_accept_only_the_final_epoch_suffix() {
    let mut app = app();
    ingress(&mut app, 1, clear_recipes());
    complete_empty_grid(&mut app, 2);
    app.world_mut()
        .resource_mut::<ClientWorld>()
        .stream
        .as_mut()
        .unwrap()
        .submit(
            7,
            WorldEvent::ChangeDimension(protocol::ChangeDimensionEvent {
                dimension: 1,
                position: [0.0, 70.0, 0.0],
            }),
        )
        .unwrap();
    ingress(&mut app, 8, empty_slot(13, 28));
    app.update();
    assert!(
        app.world()
            .resource::<UiRuntime>()
            .crafting_preview()
            .is_none()
    );
    for (sequence, dimension) in [(9, 0), (10, 0)] {
        app.world_mut()
            .resource_mut::<ClientWorld>()
            .stream
            .as_mut()
            .unwrap()
            .submit(
                sequence,
                WorldEvent::ChangeDimension(protocol::ChangeDimensionEvent {
                    dimension,
                    position: [0.0, 70.0, 0.0],
                }),
            )
            .unwrap();
    }
    complete_empty_grid(&mut app, 11);
    app.update();
    assert_eq!(
        app.world()
            .resource::<ClientWorld>()
            .stream
            .as_ref()
            .unwrap()
            .form_dimension_epoch(),
        10
    );
    assert_eq!(
        app.world().resource::<UiRuntime>().crafting_preview(),
        Some(CraftingPreview::NoMatch)
    );
}

#[test]
fn legacy_destructive_pop_remains_destructive_and_partial_updates_cannot_restore_crafting() {
    let mut app = app();
    ingress(&mut app, 1, clear_recipes());
    complete_empty_grid(&mut app, 2);
    app.update();
    assert!(
        app.world()
            .resource::<UiRuntime>()
            .crafting_preview()
            .is_some()
    );
    ingress(&mut app, 7, empty_slot(12, 28));
    {
        let mut runtime = app.world_mut().resource_mut::<UiRuntime>();
        assert_eq!(runtime.pop_inventory_event().unwrap().fifo_sequence, 7);
        assert!(runtime.pop_inventory_event().is_none());
        assert!(runtime.crafting_preview().is_none());
    }
    ingress(&mut app, 8, empty_slot(13, 28));
    app.update();
    assert!(
        app.world()
            .resource::<UiRuntime>()
            .crafting_preview()
            .is_none()
    );
    assert!(matches!(
        app.world()
            .resource::<UiRuntime>()
            .inventory_ledger()
            .slot_state(28),
        Some(PlayerInventorySlot::Unknown)
    ));
}

#[test]
fn ordinary_transfer_bytes_and_conservation_are_identical_after_craft_only_overflow() {
    fn transfer(overflow: bool) -> (Vec<u8>, u16, u16) {
        use crate::ui_runtime::inventory_ledger::{
            PERSONAL_INVENTORY_WINDOW_TYPE, PLAYER_INVENTORY_SLOT_COUNT,
        };
        let mut app = app();
        let registry = ItemRegistryEvent {
            entries: std::sync::Arc::from([protocol::ItemRegistryEntry {
                identifier: "minecraft:apple".into(),
                network_id: 878,
                component_based: true,
                version: protocol::ItemRegistryVersion::DataDriven,
                component_digest: [8; 32],
                negotiated_max_stack_size: Some(64),
                canonical_empty_component_data: false,
            }]),
        };
        assert!(publish_bootstrap_inventory(
            &mut app.world_mut().resource_mut::<UiRuntime>(),
            Some(registry),
            InventoryEvent::Authority(InventoryAuthority::Server)
        ));
        let stack = |id, count| NetworkItemStack {
            network_id: 878,
            stack_network_id: id,
            count,
            ..NetworkItemStack::default()
        };
        let mut slots = vec![NetworkItemStack::empty(); PLAYER_INVENTORY_SLOT_COUNT];
        slots[0] = stack(60, 60);
        ingress(
            &mut app,
            1,
            InventoryEvent::Content(protocol::InventoryContentEvent {
                container: ContainerIdentity::window(0),
                slots: slots.into(),
                storage_item: NetworkItemStack::empty(),
            }),
        );
        ingress(
            &mut app,
            2,
            InventoryEvent::Content(protocol::InventoryContentEvent {
                container: ContainerIdentity {
                    window_id: Some(-1),
                    slot_type: Some(59),
                    dynamic_id: None,
                },
                slots: std::sync::Arc::from([stack(33, 33)]),
                storage_item: NetworkItemStack::empty(),
            }),
        );
        app.update();
        if overflow {
            for sequence in 4..=67 {
                ingress(&mut app, sequence, empty_slot(13, 28));
                app.update();
            }
            app.world_mut()
                .resource_mut::<ClientWorld>()
                .stream
                .as_mut()
                .unwrap()
                .commit(3)
                .unwrap();
            ingress(&mut app, 68, empty_slot(13, 28));
            app.update();
            assert!(
                app.world()
                    .resource::<UiRuntime>()
                    .crafting_preview()
                    .is_none()
            );
            assert!(app.world().resource::<ClientWorld>().fatal_error.is_none());
        }
        let mut runtime = app.world_mut().resource_mut::<UiRuntime>();
        let ledger = runtime.inventory_ledger_mut();
        assert!(ledger.request_personal_open(42));
        assert!(ledger.mark_transport_enqueued(0));
        ledger.apply(&InventoryEvent::Open(protocol::ContainerOpenEvent {
            container: ContainerIdentity::window(2),
            window_type: PERSONAL_INVENTORY_WINDOW_TYPE,
            position: [0, 64, 0],
            runtime_entity_id: -1,
        }));
        assert_eq!(ledger.begin_click(0), Ok(-3));
        let mut encoded = bytes::BytesMut::new();
        ledger
            .pending_packet()
            .unwrap()
            .unwrap()
            .encode_bytes_mut(&mut encoded)
            .unwrap();
        let player = ledger.displayed_stack(0).unwrap().count;
        let cursor = ledger.cursor_stack().unwrap().count;
        assert_eq!(player + cursor, 93);
        (encoded.to_vec(), player, cursor)
    }
    assert_eq!(transfer(false), transfer(true));
}
