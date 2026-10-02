use protocol::{
    ContainerIdentity, InventoryEvent, InventorySlotEvent, ItemRegistryEntry, ItemRegistryEvent,
    ItemRegistryVersion, SlotIdentity,
};

use super::*;
use crate::item_use::crossbow as native_crossbow;

const ARROW: i32 = BOW + 1;
const FIREWORK: i32 = BOW + 2;

fn stack(network_id: i32) -> NetworkItemStack {
    NetworkItemStack {
        network_id,
        count: 1,
        stack_network_id: 41,
        ..NetworkItemStack::default()
    }
}

fn charged_stack() -> NetworkItemStack {
    let mut extra = vec![255, 255, 1, 10, 0, 0, 10, 11, 0];
    extra.extend_from_slice(b"chargedItem");
    extra.extend_from_slice(&[8, 4, 0]);
    extra.extend_from_slice(b"Name");
    let name = b"minecraft:arrow";
    extra.extend_from_slice(&(name.len() as u16).to_le_bytes());
    extra.extend_from_slice(name);
    extra.extend_from_slice(&[0, 0]);
    NetworkItemStack {
        nbt_digest: Sha256::digest(&extra).into(),
        extra_data: extra.into(),
        ..stack(BOW)
    }
}

fn fixture() -> (WorldStream, UiRuntime) {
    let mut stream = WorldStream::new(protocol::WorldBootstrap {
        dimension: 0,
        local_player_runtime_id: 1,
        local_player_unique_id: 1,
        player_position: [0.0; 3],
        world_spawn_position: [0; 3],
        air_network_id: protocol::SEQUENTIAL_AIR_NETWORK_ID,
        block_network_ids_are_hashes: false,
    });
    assert!(
        stream.seed_item_registry(ItemRegistryEvent {
            entries: [
                (BOW, "minecraft:crossbow"),
                (ARROW, "minecraft:arrow"),
                (FIREWORK, "minecraft:firework_rocket"),
            ]
            .map(|(network_id, identifier)| ItemRegistryEntry {
                identifier: identifier.into(),
                network_id,
                component_based: false,
                version: ItemRegistryVersion::Legacy,
                component_digest: [0; 32],
                negotiated_max_stack_size: None,
                canonical_empty_component_data: true,
                item_tags: Arc::from([]),
            })
            .into(),
        })
    );
    let mut ui = UiRuntime::new(1);
    ui.set_local_selected_slot(2);
    publish(&mut ui, 1, ContainerIdentity::window(0), 2, stack(BOW));
    (stream, ui)
}

fn publish(
    ui: &mut UiRuntime,
    sequence: u64,
    container: ContainerIdentity,
    slot: u16,
    stack: NetworkItemStack,
) {
    ui.enqueue_inventory_event(
        1,
        sequence,
        InventoryEvent::Slot(InventorySlotEvent {
            identity: SlotIdentity { container, slot },
            stack,
            storage_item: None,
        }),
    )
    .unwrap();
    ui.drain_pending_inventory();
}

fn use_frame(stream: &WorldStream, ui: &UiRuntime, tick: u64) -> UseFrame {
    UseFrame {
        selection: verified_use_selection(ui),
        air_use: selected_air_use(stream, ui),
        inventory_revision: ui.inventory_ledger().authoritative_slot_revision(2),
        charge_projectile: native_crossbow::loading_projectile(stream, ui, true),
        ..frame(tick, true)
    }
}

#[test]
fn presentation_load_fire_and_authoritative_nbt_share_one_charge_state() {
    let (stream, mut ui) = fixture();
    let mut runtime = ItemUseRuntime::default();
    runtime.observe_press(true);
    runtime.step(&use_frame(&stream, &ui, 10));
    runtime.step(&use_frame(&stream, &ui, 35));
    let input = runtime.render_input(&stream, &ui, 36, 0.5);
    assert!(input.hand_charged);
    assert_eq!(input.animation_frame, 4);
    assert_eq!(input.use_elapsed_ticks, None);
    assert_eq!(input.max_use_ticks, crossbow_duration());
    assert_eq!(runtime.local_item_use(&stream, &ui), LocalItemUse::Idle);

    // Even an identical authoritative restatement rejects the local loaded state.
    publish(&mut ui, 2, ContainerIdentity::window(0), 2, stack(BOW));
    let input = runtime.render_input(&stream, &ui, 37, 0.0);
    assert!(!input.hand_charged);
    assert_eq!(input.animation_frame, 0);
    publish(&mut ui, 3, ContainerIdentity::window(0), 2, charged_stack());
    let input = runtime.render_input(&stream, &ui, 38, 0.0);
    assert!(input.hand_charged);
    assert_eq!(input.animation_frame, 4);
    assert_eq!(input.max_use_ticks, crossbow_duration());
    assert_eq!(runtime.local_item_use(&stream, &ui), LocalItemUse::Idle);

    runtime.observe_press(true);
    let fired = runtime.step(&use_frame(&stream, &ui, 39));
    assert_eq!(kinds(&fired), ["use"]);
    assert!(!fired.started);
    let input = runtime.render_input(&stream, &ui, 39, 0.0);
    assert!(!input.hand_charged);
    assert_eq!(input.animation_frame, 0);
    publish(&mut ui, 4, ContainerIdentity::window(0), 2, stack(BOW));
    assert!(!runtime.render_input(&stream, &ui, 40, 0.0).hand_charged);
}

#[test]
fn offhand_projectile_precedes_inventory_and_only_creative_synthesizes_ammo() {
    let (stream, mut ui) = fixture();
    assert_eq!(
        native_crossbow::loading_projectile(&stream, &ui, false),
        None
    );
    assert_eq!(
        native_crossbow::loading_projectile(&stream, &ui, true),
        Some("minecraft:arrow")
    );
    publish(&mut ui, 2, ContainerIdentity::window(0), 5, stack(ARROW));
    assert_eq!(
        native_crossbow::loading_projectile(&stream, &ui, false),
        Some("minecraft:arrow")
    );
    let offhand = ContainerIdentity {
        window_id: None,
        slot_type: Some(protocol::CONTAINER_NAME_OFFHAND),
        dynamic_id: None,
    };
    publish(&mut ui, 3, offhand, 0, stack(FIREWORK));
    assert_eq!(
        native_crossbow::loading_projectile(&stream, &ui, false),
        Some("minecraft:firework_rocket")
    );
    assert_eq!(
        native_crossbow::loading_projectile(&stream, &ui, true),
        Some("minecraft:firework_rocket")
    );
    publish(&mut ui, 4, offhand, 0, stack(ARROW));
    assert_eq!(
        native_crossbow::loading_projectile(&stream, &ui, false),
        Some("minecraft:arrow")
    );
}

#[test]
fn normal_transaction_clears_loaded_prediction_and_retains_charged_nbt_and_offhand() {
    let (stream, mut ui) = fixture();
    let mut runtime = ItemUseRuntime::default();
    runtime.observe_press(true);
    runtime.step(&use_frame(&stream, &ui, 10));
    runtime.step(&use_frame(&stream, &ui, 35));
    assert!(runtime.render_input(&stream, &ui, 36, 0.0).hand_charged);
    let batch = |main, offhand| {
        InventoryEvent::Transaction(protocol::InventoryTransactionEvent {
            slots: Arc::from([
                InventorySlotEvent {
                    identity: SlotIdentity {
                        container: ContainerIdentity::window(protocol::PLAYER_INVENTORY_WINDOW_ID),
                        slot: 2,
                    },
                    stack: main,
                    storage_item: None,
                },
                InventorySlotEvent {
                    identity: SlotIdentity {
                        container: ContainerIdentity::window(protocol::OFFHAND_WINDOW_ID),
                        slot: 0,
                    },
                    stack: offhand,
                    storage_item: None,
                },
            ]),
            skipped_actions: 0,
        })
    };
    ui.enqueue_inventory_event(1, 2, batch(stack(BOW), stack(FIREWORK)))
        .unwrap();
    ui.drain_pending_inventory();
    assert!(!runtime.render_input(&stream, &ui, 37, 0.0).hand_charged);
    assert_eq!(
        native_crossbow::loading_projectile(&stream, &ui, false),
        Some("minecraft:firework_rocket")
    );
    assert_eq!(ui.gameplay_hud().offhand_stack(), Some(&stack(FIREWORK)));
    ui.enqueue_inventory_event(1, 3, batch(charged_stack(), stack(ARROW)))
        .unwrap();
    ui.drain_pending_inventory();
    let rendered = runtime.render_input(&stream, &ui, 38, 0.0);
    assert!(rendered.hand_charged);
    assert_eq!(rendered.animation_frame, 4);
    assert_eq!(ui.gameplay_hud().hotbar_stack(2), Some(&charged_stack()));
}
