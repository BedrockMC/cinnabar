use super::*;
use protocol::{ContainerIdentity, InventorySlotEvent, NetworkItemStack, SlotIdentity};

fn slot(name: u8, slot: u16) -> InventoryAuthorityEvent {
    InventoryAuthorityEvent::Inventory(InventoryEvent::Slot(InventorySlotEvent {
        identity: SlotIdentity {
            container: ContainerIdentity {
                window_id: Some(124),
                slot_type: Some(name),
                dynamic_id: None,
            },
            slot,
        },
        stack: NetworkItemStack::empty(),
        storage_item: None,
    }))
}

fn named_registry(name: &str, capacity: u8) -> protocol::ItemRegistryEvent {
    protocol::ItemRegistryEvent {
        entries: Arc::from([protocol::ItemRegistryEntry {
            identifier: Arc::from(name),
            network_id: 6,
            component_based: false,
            version: protocol::ItemRegistryVersion::None,
            component_digest: [0; 32],
            negotiated_max_stack_size: Some(capacity),
            canonical_empty_component_data: true,
        }]),
    }
}

fn present_slot(name: u8, position: u16) -> InventoryAuthorityEvent {
    let mut event = slot(name, position);
    let InventoryAuthorityEvent::Inventory(InventoryEvent::Slot(update)) = &mut event else {
        unreachable!()
    };
    update.stack = NetworkItemStack {
        network_id: 6,
        stack_network_id: 101,
        count: 1,
        ..NetworkItemStack::empty()
    };
    event
}

#[test]
fn registry_rebinding_retires_prior_position_cells_without_mutating_an_older_clone() {
    let mut state = CraftingAuthority::new(1);
    state.observe(
        1,
        1,
        &InventoryAuthorityEvent::Registry(named_registry("minecraft:oak_log", 64)),
    );
    state.observe(1, 2, &present_slot(13, 28));
    state.observe(1, 3, &present_slot(59, 0));
    state.synchronize(Some((1, 0, Some(3))));
    state.advance();
    let old = state.clone();
    state.observe(
        1,
        4,
        &InventoryAuthorityEvent::Registry(named_registry("minecraft:birch_log", 64)),
    );
    state.synchronize(Some((1, 0, Some(4))));
    state.advance();
    assert!(
        state.grid[0].is_none(),
        "a reused numeric ID must not reinterpret an older cell"
    );
    assert!(state.cursor.is_none());
    assert!(old.grid[0].is_some());
    assert!(old.cursor.is_some());
    state.observe(1, 5, &present_slot(13, 28));
    state.synchronize(Some((1, 0, Some(5))));
    state.advance();
    assert!(
        state.grid[0].is_some(),
        "a genuinely newer cell uses the replacement binding"
    );
}

#[test]
fn initial_registry_cannot_retroactively_bind_an_unknown_item_but_capacity_updates_keep_identity() {
    let mut state = CraftingAuthority::new(1);
    state.observe(1, 1, &present_slot(13, 28));
    state.observe(
        1,
        2,
        &InventoryAuthorityEvent::Registry(named_registry("minecraft:oak_log", 64)),
    );
    state.synchronize(Some((1, 0, Some(2))));
    state.advance();
    assert!(state.grid[0].is_none());
    state.observe(1, 3, &present_slot(13, 28));
    state.synchronize(Some((1, 0, Some(3))));
    state.advance();
    let cell = Arc::clone(state.grid[0].as_ref().unwrap());
    state.observe(
        1,
        4,
        &InventoryAuthorityEvent::Registry(named_registry("minecraft:oak_log", 1)),
    );
    state.synchronize(Some((1, 0, Some(4))));
    state.advance();
    assert!(Arc::ptr_eq(&cell, state.grid[0].as_ref().unwrap()));
    assert_eq!(
        state
            .registry
            .as_ref()
            .unwrap()
            .snapshot
            .get(6)
            .unwrap()
            .negotiated_max_stack_size,
        Some(1)
    );
}

fn authority(mode: InventoryAuthority) -> InventoryAuthorityEvent {
    InventoryAuthorityEvent::Inventory(InventoryEvent::Authority(mode))
}

#[test]
fn immediate_client_revocation_cannot_be_undone_by_an_older_committed_server_or_bootstrap() {
    let mut state = CraftingAuthority::new(1);
    state.synchronize(Some((1, 0, Some(1))));
    state.observe(1, 1, &authority(InventoryAuthority::Server));
    state.observe(1, 2, &authority(InventoryAuthority::Client));
    state.bootstrap(None, InventoryAuthority::Server);
    state.advance();
    assert_eq!(state.authority, None);
    assert_eq!(state.authority_loss, 2);
    state.synchronize(Some((1, 0, Some(2))));
    state.advance();
    assert_eq!(state.authority, None);
    state.observe(1, 3, &authority(InventoryAuthority::Server));
    state.synchronize(Some((1, 0, Some(3))));
    state.advance();
    assert_eq!(state.authority, Some(InventoryAuthority::Server));
    assert!(state.grid.iter().all(Option::is_none));
    assert!(state.cursor.is_none());
}

#[test]
fn final_dimension_epoch_discards_old_cells_but_preserves_new_cells_and_unknown_cursor() {
    let mut state = CraftingAuthority::new(1);
    state.observe(1, 1, &slot(13, 28));
    state.observe(1, 3, &slot(13, 29));
    state.observe(1, 5, &slot(13, 30));
    state.synchronize(Some((1, 4, Some(5))));
    state.advance();
    assert!(state.grid[0].is_none());
    assert!(state.grid[1].is_none());
    assert!(state.grid[2].is_some());
    assert!(state.grid[3].is_none());
    assert!(state.cursor.is_none());
    assert!(state.preview.is_none());
}

#[test]
fn overflow_is_craft_only_and_partial_facts_cannot_recover_past_its_barrier() {
    let mut state = CraftingAuthority::new(1);
    state.bootstrap(None, InventoryAuthority::Server);
    state.synchronize(Some((1, 0, Some(0))));
    for sequence in 1..=65 {
        state.observe(1, sequence, &slot(13, 28));
    }
    assert!(state.queue.is_none());
    assert_eq!(state.barrier, 65);
    assert_eq!(state.authority, Some(InventoryAuthority::Server));
    state.observe(1, 66, &slot(13, 28));
    state.synchronize(Some((1, 0, Some(66))));
    state.advance();
    assert!(state.grid[0].is_some());
    assert!(state.grid[1..].iter().all(Option::is_none));
    assert!(state.cursor.is_none());
    assert!(state.preview.is_none());
}

#[test]
fn cloned_snapshots_share_queue_owners_and_updates_do_not_mutate_the_old_snapshot() {
    let mut state = CraftingAuthority::new(1);
    state.observe(1, 1, &slot(13, 28));
    let old = state.clone();
    assert!(Arc::ptr_eq(
        old.queue.as_ref().unwrap(),
        state.queue.as_ref().unwrap()
    ));
    state.observe(1, 2, &slot(13, 29));
    assert_eq!(old.queue.as_ref().unwrap().records.len(), 1);
    assert_eq!(state.queue.as_ref().unwrap().records.len(), 2);
    assert!(!Arc::ptr_eq(
        old.queue.as_ref().unwrap(),
        state.queue.as_ref().unwrap()
    ));
}

#[test]
fn registry_projection_uses_event_position_and_invalid_replacement_retires_only_craft_binding() {
    let a = protocol::ItemRegistryEvent {
        entries: protocol::vanilla_item_registry(),
    };
    let mut changed = a.entries.to_vec();
    changed[0].negotiated_max_stack_size = Some(1);
    let b = protocol::ItemRegistryEvent {
        entries: changed.into(),
    };
    let mut state = CraftingAuthority::new(1);
    state.observe(1, 1, &InventoryAuthorityEvent::Registry(a.clone()));
    state.observe(1, 2, &slot(13, 28));
    state.observe(1, 3, &InventoryAuthorityEvent::Registry(b.clone()));
    state.synchronize(Some((1, 0, Some(1))));
    state.advance();
    assert!(std::ptr::eq(
        state.registry.as_ref().unwrap().snapshot.entries().as_ptr(),
        a.entries.as_ptr()
    ));
    assert!(state.grid[0].is_none());
    let old = state.clone();
    state.synchronize(Some((1, 0, Some(3))));
    state.advance();
    assert!(std::ptr::eq(
        state.registry.as_ref().unwrap().snapshot.entries().as_ptr(),
        b.entries.as_ptr()
    ));
    assert!(state.grid[0].is_some());
    assert!(std::ptr::eq(
        old.registry.as_ref().unwrap().snapshot.entries().as_ptr(),
        a.entries.as_ptr()
    ));
    let generation = state.registry.as_ref().unwrap().snapshot.revision();
    let duplicate = protocol::ItemRegistryEvent {
        entries: Arc::from([b.entries[0].clone(), b.entries[0].clone()]),
    };
    state.observe(1, 4, &InventoryAuthorityEvent::Registry(duplicate));
    assert!(state.registry.is_none());
    assert!(state.grid.iter().all(Option::is_none));
    state.observe(1, 5, &InventoryAuthorityEvent::Registry(b));
    state.synchronize(Some((1, 0, Some(5))));
    state.advance();
    assert!(state.registry.as_ref().unwrap().snapshot.revision() > generation);
}

#[test]
fn runtime_clones_share_credited_authority_without_sharing_mutable_state() {
    let mut runtime = crate::ui_runtime::UiRuntime::new(1);
    runtime.crafting_authority.observe(
        1,
        1,
        &InventoryAuthorityEvent::Registry(protocol::ItemRegistryEvent {
            entries: protocol::vanilla_item_registry(),
        }),
    );
    runtime
        .crafting_authority
        .synchronize(Some((1, 0, Some(1))));
    runtime.crafting_authority.advance();
    let old = runtime.clone();
    assert!(Arc::ptr_eq(
        old.crafting_authority.registry.as_ref().unwrap(),
        runtime.crafting_authority.registry.as_ref().unwrap()
    ));
    runtime.crafting_authority.bypass(2);
    assert!(runtime.crafting_authority.registry.is_none());
    assert!(old.crafting_authority.registry.is_some());
}

#[test]
fn absent_replaced_stale_or_wrong_session_receipts_cannot_revive_retired_authority() {
    let mut state = CraftingAuthority::new(1);
    state.bootstrap(None, InventoryAuthority::Server);
    state.synchronize(Some((1, 0, Some(1))));
    state.observe(1, 1, &slot(13, 28));
    state.advance();
    assert!(state.grid[0].is_some());
    state.synchronize(None);
    assert!(state.grid[0].is_none());
    assert!(state.authority.is_none());
    state.bootstrap(None, InventoryAuthority::Server);
    assert!(state.authority.is_none());
    state.observe(2, 100, &authority(InventoryAuthority::Server));
    assert_eq!(state.observed, 1);
    state.synchronize(Some((2, 0, Some(0))));
    assert!(state.authority.is_none());
    state.observe(1, 2, &slot(13, 28));
    state.synchronize(Some((2, 3, Some(2))));
    assert!(state.through.is_none());
    assert!(state.queue.is_none());
    assert!(state.grid.iter().all(Option::is_none));
}

#[test]
fn invalid_registry_does_not_change_ordinary_lookup_and_identical_valid_event_restores_binding() {
    let mut runtime = crate::ui_runtime::UiRuntime::new(1);
    let registry = protocol::ItemRegistryEvent {
        entries: protocol::vanilla_item_registry(),
    };
    let id = registry.entries[0].network_id;
    runtime.synchronize_crafting_frontier(1, Some((1, 0, Some(1))));
    runtime
        .enqueue_item_registry_event(1, 1, registry.clone())
        .unwrap();
    runtime.drain_pending_inventory();
    let generation = runtime
        .crafting_authority
        .registry
        .as_ref()
        .unwrap()
        .snapshot
        .revision();
    let retained = runtime
        .inventory_ledger
        .negotiated_item_entry(id)
        .unwrap()
        .clone();
    let duplicate = protocol::ItemRegistryEvent {
        entries: Arc::from([registry.entries[0].clone(), registry.entries[0].clone()]),
    };
    runtime
        .enqueue_item_registry_event(1, 2, duplicate)
        .unwrap();
    runtime.drain_pending_inventory();
    assert_eq!(
        runtime.inventory_ledger.negotiated_item_entry(id),
        Some(&retained)
    );
    assert!(runtime.crafting_authority.registry.is_none());
    runtime.enqueue_item_registry_event(1, 3, registry).unwrap();
    runtime.synchronize_crafting_frontier(1, Some((1, 0, Some(3))));
    runtime.drain_pending_inventory();
    assert_eq!(
        runtime.inventory_ledger.negotiated_item_entry(id),
        Some(&retained)
    );
    assert!(
        runtime
            .crafting_authority
            .registry
            .as_ref()
            .unwrap()
            .snapshot
            .revision()
            > generation
    );
}
