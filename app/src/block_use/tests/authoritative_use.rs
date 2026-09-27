use super::*;
use protocol::{ItemRegistryEntry, ItemRegistryEvent, ItemRegistryVersion};

fn entry(identifier: &str) -> ItemRegistryEntry {
    ItemRegistryEntry {
        identifier: Arc::from(identifier),
        network_id: 2,
        component_based: false,
        version: ItemRegistryVersion::Legacy,
        component_digest: [0; 32],
        negotiated_max_stack_size: None,
        canonical_empty_component_data: true,
    }
}

fn selected(entry: ItemRegistryEntry, runtime_id: u32) -> UiRuntime {
    let mut ui = UiRuntime::new(7);
    ui.publish_player_game_mode(protocol::PlayerGameMode::Creative);
    ui.set_local_selected_slot(2);
    ui.inventory_ledger_mut()
        .apply_registry(&ItemRegistryEvent {
            entries: Arc::from([entry]),
        });
    let mut stack = network_item(2);
    stack.count = 37;
    stack.metadata = 3;
    stack.block_runtime_id = i32::from_ne_bytes(runtime_id.to_ne_bytes());
    stack.extra_data = Arc::from([0; 10]);
    stack.nbt_digest = Sha256::digest(&stack.extra_data).into();
    ui.inventory_ledger_mut().apply(&inventory_slot(2, stack));
    ui
}

fn records() -> Box<[RegistryRecord]> {
    read_registry_for_protocol(
        include_bytes!("../../../../crates/assets/data/block-registry-v2168.bin"),
        2168,
    )
    .unwrap()
}

#[test]
fn negotiated_cube_selection_preserves_data_and_signed_hash_bits() {
    let records = records();
    let collisions = fixture_registries();
    for (mode, version) in [
        (
            assets::NetworkIdMode::Sequential,
            ItemRegistryVersion::Legacy,
        ),
        (assets::NetworkIdMode::Sequential, ItemRegistryVersion::None),
        (assets::NetworkIdMode::Hashed, ItemRegistryVersion::Legacy),
        (assets::NetworkIdMode::Hashed, ItemRegistryVersion::None),
    ] {
        let record = records
            .iter()
            .find(|r| {
                let id = match mode {
                    assets::NetworkIdMode::Sequential => r.sequential_id,
                    assets::NetworkIdMode::Hashed => r.network_hash,
                };
                collisions.interaction_cube(mode, id).is_some()
                    && id != 0
                    && (mode == assets::NetworkIdMode::Sequential || id > i32::MAX as u32)
            })
            .unwrap();
        let id = match mode {
            assets::NetworkIdMode::Sequential => record.sequential_id,
            assets::NetworkIdMode::Hashed => record.network_hash,
        };
        let mut definition = entry(&record.name);
        definition.version = version;
        let ui = selected(definition, id);
        let expected = ui.selected_stack().unwrap().clone();
        let selection =
            crate::block_use::verified_block_use_selection(&ui, &collisions, mode).unwrap();
        assert_eq!(selection.slot, 2);
        assert_eq!(
            selection.item,
            VerifiedNetworkItemStack::try_new(expected.clone(), expected.nbt_digest).unwrap()
        );
        assert_eq!(
            u32::from_ne_bytes(selection.item.block_runtime_id().to_ne_bytes()),
            id
        );
        assert_eq!(ui.selected_stack(), Some(&expected));
    }
}

#[test]
fn known_none_remains_distinct_from_unsupported_versions_and_requires_exact_authority() {
    for version in [ItemRegistryVersion::Legacy, ItemRegistryVersion::None] {
        reject_ambiguous_selection(version);
    }
}

fn reject_ambiguous_selection(version: ItemRegistryVersion) {
    let records = records();
    let collisions = fixture_registries();
    let record = records
        .iter()
        .find(|r| {
            r.sequential_id != 0
                && collisions
                    .interaction_cube(assets::NetworkIdMode::Sequential, r.sequential_id)
                    .is_some()
        })
        .unwrap();
    let mut base = entry(&record.name);
    base.version = version;
    for version in [
        ItemRegistryVersion::DataDriven,
        ItemRegistryVersion::Unknown(99),
    ] {
        let mut candidate = base.clone();
        candidate.version = version;
        assert!(
            crate::block_use::verified_block_use_selection(
                &selected(candidate, record.sequential_id),
                &collisions,
                assets::NetworkIdMode::Sequential
            )
            .is_none()
        );
    }
    for change in 0..3 {
        let mut candidate = base.clone();
        match change {
            0 => candidate.component_based = true,
            1 => candidate.canonical_empty_component_data = false,
            _ => candidate.identifier = Arc::from("fixture:unbound"),
        }
        assert!(
            crate::block_use::verified_block_use_selection(
                &selected(candidate, record.sequential_id),
                &collisions,
                assets::NetworkIdMode::Sequential
            )
            .is_none()
        );
    }
    for id in [0, u32::MAX] {
        assert!(
            crate::block_use::verified_block_use_selection(
                &selected(base.clone(), id),
                &collisions,
                assets::NetworkIdMode::Sequential
            )
            .is_none()
        );
    }
    for change in 0..4 {
        let mut ui = selected(base.clone(), record.sequential_id);
        let mut stack = ui.selected_stack().unwrap().clone();
        match change {
            0 => stack.stack_network_id = -1,
            1 => stack.nbt_digest = [0; 32],
            2 => stack.network_id = 99,
            _ => stack.stack_network_id = 0,
        }
        ui.inventory_ledger_mut().apply(&inventory_slot(2, stack));
        assert!(
            crate::block_use::verified_block_use_selection(
                &ui,
                &collisions,
                assets::NetworkIdMode::Sequential
            )
            .is_none()
        );
    }
    let mut pending = selected(base.clone(), record.sequential_id);
    pending.queue_local_hotbar_selection(3);
    assert!(
        crate::block_use::verified_block_use_selection(
            &pending,
            &collisions,
            assets::NetworkIdMode::Sequential
        )
        .is_none()
    );
    let mut absent = selected(base, record.sequential_id);
    absent.begin_session(8);
    assert!(
        crate::block_use::verified_block_use_selection(
            &absent,
            &collisions,
            assets::NetworkIdMode::Sequential
        )
        .is_none()
    );
}

#[test]
fn full_cube_support_does_not_admit_special_shapes_or_item_aliases() {
    for version in [ItemRegistryVersion::Legacy, ItemRegistryVersion::None] {
        reject_special_shape(version);
    }
}

fn reject_special_shape(version: ItemRegistryVersion) {
    let records = records();
    let collisions = fixture_registries();
    let special = records
        .iter()
        .find(|r| r.name.as_ref() == "minecraft:chest")
        .unwrap();
    let mut definition = entry(&special.name);
    definition.version = version;
    let ui = selected(definition, special.sequential_id);
    assert!(
        crate::block_use::verified_block_use_selection(
            &ui,
            &collisions,
            assets::NetworkIdMode::Sequential
        )
        .is_none()
    );
}

#[test]
fn filled_pending_and_recovery_never_use_predicted_inventory() {
    for version in [ItemRegistryVersion::Legacy, ItemRegistryVersion::None] {
        reject_pending_and_recovery(version);
    }
}

fn reject_pending_and_recovery(version: ItemRegistryVersion) {
    let records = records();
    let collisions = fixture_registries();
    let cube = records
        .iter()
        .find(|r| {
            r.sequential_id != 0
                && collisions
                    .interaction_cube(assets::NetworkIdMode::Sequential, r.sequential_id)
                    .is_some()
        })
        .unwrap();
    let mut definition = entry(&cube.name);
    definition.version = version;
    let mut ui = selected(definition, cube.sequential_id);
    ui.inventory_ledger_mut()
        .apply(&InventoryEvent::Authority(InventoryAuthority::Server));
    admit_personal_inventory(&mut ui);
    ui.inventory_ledger_mut()
        .apply(&inventory_slot(3, network_item(3)));
    ui.inventory_ledger_mut().begin_click(3).unwrap();
    assert!(
        crate::block_use::verified_block_use_selection(
            &ui,
            &collisions,
            assets::NetworkIdMode::Sequential
        )
        .is_none()
    );
    assert!(ui.inventory_ledger_mut().mark_transport_enqueued(0));
    ui.inventory_ledger_mut().poll_timeout(u64::MAX);
    assert!(ui.inventory_ledger().resync_required());
    assert!(
        crate::block_use::verified_block_use_selection(
            &ui,
            &collisions,
            assets::NetworkIdMode::Sequential
        )
        .is_none()
    );
}

#[test]
fn filled_use_stale_authority_is_stripped_without_removing_movement() {
    let mut candidate = observation(101, 2);
    let mut stack = network_item(2);
    stack.block_runtime_id = 9;
    candidate.selection.item =
        VerifiedNetworkItemStack::try_new(stack.clone(), stack.nbt_digest).unwrap();
    let original = FrozenBlockUse::from_observation(candidate).unwrap();
    for change in 0..7 {
        let mut current = original.clone();
        match change {
            0 => current.observation.frame.session_generation += 1,
            1 => current.observation.frame.position_authority_generation += 1,
            2 => current.observation.frame.input_authority_generation = NonZeroU64::new(6).unwrap(),
            3 => current.observation.selection.slot += 1,
            4 => current.observation.target.runtime_id += 1,
            5 => current.observation.ray.world_identity = world_identity(4),
            _ => current.game_mode = protocol::PlayerGameMode::Survival,
        }
        let mut ticker = ticker_with_tick();
        assert_eq!(ticker.attach_block_use(original.clone()), Some(101));
        ticker.retain_block_use(Some(&current));
        assert!(!ticker.has_queued_block_use());
        let mut sent = 0;
        flush_player_auth_inputs(&mut ticker, 1, Some(evidence()), |_, _| {
            sent += 1;
            Ok::<_, &str>(())
        })
        .unwrap();
        assert_eq!(sent, 1);
    }
}

fn production_world(runtime_id: u32) -> crate::runtime::world::ClientWorld {
    let mut stream = client_world::WorldStream::new(protocol::WorldBootstrap {
        local_player_unique_id: 1,
        local_player_runtime_id: 1,
        dimension: 0,
        player_position: [0.5, -62.5, 0.5],
        world_spawn_position: [0; 3],
        air_network_id: protocol::SEQUENTIAL_AIR_NETWORK_ID,
        block_network_ids_are_hashes: false,
    });
    let mut payload = vec![9, 1, (-4_i8) as u8, 1];
    let mut encoded = protocol::SEQUENTIAL_AIR_NETWORK_ID << 1;
    loop {
        let byte = (encoded & 0x7f) as u8;
        encoded >>= 7;
        payload.push(byte | if encoded == 0 { 0 } else { 0x80 });
        if encoded == 0 {
            break;
        }
    }
    payload.extend([1, 2]);
    payload.extend(std::iter::repeat_n(0xff, 23));
    payload.push(0);
    stream
        .submit(
            1,
            protocol::WorldEvent::LevelChunk(protocol::LevelChunkEvent {
                dimension: 0,
                x: 0,
                z: 0,
                mode: protocol::LevelChunkMode::Inline { count: 1 },
                payload,
            }),
        )
        .unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while stream
        .collision_store()
        .sub_chunk(world::SubChunkKey::new(0, 0, -4, 0))
        .is_none()
    {
        stream.poll([0.5, -62.5, 0.5], 0);
        assert!(stream.take_fatal_error().is_none());
        assert!(std::time::Instant::now() < deadline);
        std::thread::yield_now();
    }
    stream
        .submit(
            2,
            protocol::WorldEvent::BlockUpdates(vec![protocol::BlockUpdateEvent {
                dimension: 0,
                position: [0, -63, 2],
                network_id: runtime_id,
                layer: 0,
            }]),
        )
        .unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        stream.poll([0.5, -62.5, 0.5], 0);
        assert!(stream.take_fatal_error().is_none());
        let target_committed = stream
            .collision_store()
            .sub_chunk(world::SubChunkKey::new(0, 0, -4, 0))
            .is_some_and(|chunk| chunk.runtime_id(0, 0, 1, 2) == Some(runtime_id));
        if stream.committed_sequence() == 2 && target_committed {
            break;
        }
        assert!(std::time::Instant::now() < deadline);
        std::thread::yield_now();
    }
    crate::runtime::world::ClientWorld {
        stream: Some(stream),
        ..Default::default()
    }
}

#[test]
fn production_observation_and_tick_attachment_do_not_predict_world_or_inventory() {
    assert_eq!(
        production_use_payload(ItemRegistryVersion::Legacy),
        production_use_payload(ItemRegistryVersion::None)
    );
}

fn production_use_payload(version: ItemRegistryVersion) -> Vec<u8> {
    let records = records();
    let collisions = fixture_registries();
    let cube = records
        .iter()
        .find(|r| {
            r.sequential_id != 0
                && collisions
                    .interaction_cube(assets::NetworkIdMode::Sequential, r.sequential_id)
                    .is_some()
        })
        .unwrap();
    let world = production_world(cube.sequential_id);
    let stream = world.stream.as_ref().unwrap();
    let session = stream.actor_session_id();
    let stack = selected(entry(&cube.name), cube.sequential_id)
        .selected_stack()
        .unwrap()
        .clone();
    let mut definition = entry(&cube.name);
    definition.version = version;
    let mut ui = UiRuntime::new(session);
    ui.publish_player_game_mode(protocol::PlayerGameMode::Survival);
    ui.set_local_selected_slot(2);
    ui.inventory_ledger_mut()
        .apply_registry(&ItemRegistryEvent {
            entries: Arc::from([definition]),
        });
    ui.inventory_ledger_mut()
        .apply(&inventory_slot(2, stack.clone()));
    let palette = sim::PaletteWorld::new(
        stream.collision_store(),
        collisions.registry(stream.network_id_mode()),
        0,
    );
    let hit = palette
        .block_interaction_ray_current(
            sim::Vec3::new(0.5, -62.5, 0.5),
            sim::Vec3::new(0.0, 0.0, 1.0),
            5.7,
        )
        .unwrap()
        .unwrap();
    let mut carrier = crate::local_player::LocalPlayerFrameCarrier::default();
    carrier
        .publish(crate::local_player::LocalPlayerFrameSample {
            session_generation: session,
            fifo_sequence: stream.committed_sequence(),
            physics_tick: 101,
            perspective: semantic_input::PerspectiveMode::FirstPerson,
            world_collision_identity: hit.identity.clone(),
            pose: bevy::prelude::Transform::from_xyz(0.5, -62.5, 0.5),
            eye: bevy::prelude::Vec3::new(0.5, -62.5, 0.5),
            rotation: bevy::prelude::Quat::from_rotation_y(std::f32::consts::PI),
        })
        .unwrap();
    let mut origin = crate::local_player::InteractionOriginSnapshot::default();
    origin.publish_from_local_player_frame(&carrier);
    let mut ticker = MovementTicker::default();
    ticker.reset(session, 100, [0.5, -62.5, 0.5]);
    ticker.set_source(MovementSource::Physics);
    ticker.testing_lift_spawn_settle_gate();
    let mut sample = completed(101);
    sample.position = [0.5, -62.5, 0.5];
    sample.world_identity = hit.identity;
    ticker.enqueue_completed_physics(sample).unwrap();
    let authority = NonZeroU64::new(5).unwrap();
    let observed = crate::block_use::block_use_observation(
        &origin,
        &ui,
        &world,
        &collisions,
        semantic_input::InputMode::KeyboardMouse,
        (authority, 31),
        ticker.interaction_authority_identity().1,
    )
    .unwrap();
    assert_eq!(observed.observation.target.position, [0, -63, 2]);
    let mut runtime = BlockUseRuntime::default();
    assert_eq!(
        runtime.update_press(true, authority, Some(observed.clone()), &mut ticker),
        Some(101)
    );
    assert_eq!(
        runtime.update_press(false, authority, Some(observed.clone()), &mut ticker),
        None
    );
    let queued = observed.into_tick_payload([0.5, -62.5, 0.5]);
    let Some(BlockItemInteraction::Use(request)) = queued.interactions.block_interaction else {
        panic!("expected Use");
    };
    assert_eq!(
        request.selected_item,
        VerifiedNetworkItemStack::try_new(stack.clone(), stack.nbt_digest).unwrap()
    );
    assert_eq!(request.block_position, [0, -63, 2]);
    assert!(queued.interactions.block_actions.is_empty());
    assert_eq!(ui.selected_stack(), Some(&stack));
    assert_eq!(
        stream
            .collision_store()
            .sub_chunk(world::SubChunkKey::new(0, 0, -4, 0))
            .unwrap()
            .runtime_id(0, 0, 1, 2),
        Some(cube.sequential_id)
    );
    let mut packets = 0;
    let mut encoded = Vec::new();
    flush_player_auth_inputs(&mut ticker, 1, None, |_, packet| {
        packets += 1;
        encoded = protocol::encode(&packet, &BedrockSession { shield_item_id: 0 })
            .unwrap()
            .to_vec();
        Ok::<_, &str>(())
    })
    .unwrap();
    assert_eq!(packets, 1);
    for mode in [
        protocol::PlayerGameMode::Adventure,
        protocol::PlayerGameMode::Spectator,
        protocol::PlayerGameMode::Unknown,
    ] {
        ui.publish_player_game_mode(mode);
        assert!(
            crate::block_use::block_use_observation(
                &origin,
                &ui,
                &world,
                &collisions,
                semantic_input::InputMode::KeyboardMouse,
                (authority, 31),
                ticker.interaction_authority_identity().1
            )
            .is_none()
        );
    }
    encoded
}
