use std::sync::Arc;

use protocol::{
    ContainerIdentity, InventoryAuthority, InventoryEvent, InventorySlotEvent, ItemUseTrigger,
    NetworkItemStack, PlayerGameMode, SlotIdentity, VerifiedNetworkItemStack,
};
use sha2::{Digest, Sha256};

use super::{
    BlockUseRuntime, LocalUse, RepeatClock, UseSurroundings, placement_cell,
    placement_state_is_certain, repeat_interval_millis, use_packets, verified_use_selection,
};
use crate::{game_mode_capabilities::GameModeCapabilities, ui_runtime::UiRuntime};

fn network_item(network_id: i32, block_runtime_id: i32) -> NetworkItemStack {
    let extra_data: Arc<[u8]> = Arc::from([]);
    NetworkItemStack {
        network_id,
        metadata: 0,
        stack_network_id: 41,
        count: 1,
        nbt_digest: Sha256::digest(&extra_data).into(),
        block_runtime_id,
        extra_data,
    }
}

fn verified(stack: NetworkItemStack) -> VerifiedNetworkItemStack {
    VerifiedNetworkItemStack::try_new(stack.clone(), stack.nbt_digest).unwrap()
}

fn inventory_slot(slot: u8, stack: NetworkItemStack) -> InventoryEvent {
    InventoryEvent::Slot(InventorySlotEvent {
        identity: SlotIdentity {
            container: ContainerIdentity {
                window_id: Some(0),
                slot_type: None,
                dynamic_id: None,
            },
            slot: u16::from(slot),
        },
        stack,
        storage_item: None,
    })
}

#[test]
fn held_repeats_follow_stance_speed_and_the_survival_floor() {
    assert_eq!(repeat_interval_millis(true, false, 5.0, true), 300);
    assert_eq!(repeat_interval_millis(false, true, 5.0, true), 300);
    assert_eq!(repeat_interval_millis(false, false, 0.0, true), 200);
    assert_eq!(repeat_interval_millis(false, false, f32::NAN, false), 200);
    // Any nonzero speed uses the moving formula.
    assert_eq!(repeat_interval_millis(false, false, 0.001, true), 180);
    assert_eq!(repeat_interval_millis(false, false, 10.0, false), 90);
    assert_eq!(repeat_interval_millis(false, false, 10.0, true), 100);
}

fn clock(now_millis: u64, speed: f32) -> RepeatClock {
    RepeatClock {
        now_millis,
        sneaking: false,
        speed,
        survival: true,
    }
}

#[test]
fn press_fires_at_once_and_held_repeats_keep_a_bounded_schedule() {
    let mut runtime = BlockUseRuntime::default();
    assert_eq!(runtime.due(true, 1, clock(0, 0.0)), None);
    runtime.latched_press = true;
    let (trigger, due) = runtime.due(false, 1, clock(1_000, 0.0)).unwrap();
    assert_eq!(trigger, ItemUseTrigger::PlayerInput);
    runtime.record(trigger, due, 1, LocalUse::Place, clock(1_000, 0.0));
    // A fresh placement repeats at the slow interval, then the line is established.
    assert_eq!(runtime.due(true, 2, clock(1_300, 0.0)), None);
    let (trigger, due) = runtime.due(true, 3, clock(1_301, 0.0)).unwrap();
    assert_eq!((trigger, due), (ItemUseTrigger::SimulationTick, 1_300));
    runtime.record(trigger, due, 3, LocalUse::Place, clock(1_301, 0.0));
    // Still: anchored to now. Moving: to the due time, lagging at most 180 ms.
    assert_eq!(runtime.due(true, 4, clock(1_501, 0.0)), None);
    let (trigger, due) = runtime.due(true, 4, clock(1_502, 5.0)).unwrap();
    assert_eq!(due, 1_481);
    runtime.record(trigger, due, 4, LocalUse::Place, clock(1_502, 5.0));
    assert_eq!(runtime.last_use_millis, Some(1_481));
    runtime.record(
        ItemUseTrigger::SimulationTick,
        1_600,
        5,
        LocalUse::Place,
        clock(2_000, 5.0),
    );
    assert_eq!(runtime.last_use_millis, Some(1_820));
    // One attempt per tick; a failure keeps the schedule and retries next tick.
    assert_eq!(runtime.due(true, 5, clock(5_000, 5.0)), None);
    let (trigger, due) = runtime.due(true, 6, clock(5_000, 5.0)).unwrap();
    runtime.record(trigger, due, 6, LocalUse::Nothing, clock(5_000, 5.0));
    assert!(runtime.due(true, 7, clock(5_001, 5.0)).is_some());
}

#[test]
fn placement_targets_the_clicked_face_neighbor() {
    let clicked = [4, 64, -2];
    let cells = (0..6)
        .map(|face| placement_cell(clicked, face))
        .collect::<Vec<_>>();
    assert_eq!(
        cells,
        [
            [4, 63, -2],
            [4, 65, -2],
            [4, 64, -3],
            [4, 64, -1],
            [3, 64, -2],
            [5, 64, -2]
        ]
    );
}

fn surroundings(clicked: &str, neighbor: &str) -> UseSurroundings {
    UseSurroundings {
        clicked_identifier: Some(clicked.to_owned()),
        neighbor_identifier: Some(neighbor.to_owned()),
        player_box: ([0.2, 64.0, 0.2], [0.8, 65.8, 0.8]),
        actor_boxes: Vec::new(),
        sneaking: false,
        placed_boxes: None,
    }
}

#[test]
fn local_use_decides_interaction_placement_or_nothing() {
    let block = verified(network_item(2, 77));
    let stick = verified(network_item(3, 0));
    let empty = verified(NetworkItemStack::empty());
    let survival = GameModeCapabilities::for_mode(PlayerGameMode::Survival);
    let place = |item, clicked, face, around: &UseSurroundings| {
        LocalUse::resolve(item, clicked, face, around, &survival)
    };
    let stone = surroundings("minecraft:stone", "minecraft:air");
    assert_eq!(place(&block, [2, 63, 0], 1, &stone), LocalUse::Place);
    assert_eq!(place(&stick, [2, 63, 0], 1, &stone), LocalUse::Nothing);
    assert_eq!(
        place(
            &block,
            [2, 63, 0],
            1,
            &surroundings("minecraft:stone", "minecraft:dirt")
        ),
        LocalUse::Nothing
    );
    // The player's own column cannot receive a block, nor can an occupied cell.
    assert_eq!(place(&block, [0, 64, 0], 1, &stone), LocalUse::Nothing);
    let occupied = UseSurroundings {
        actor_boxes: vec![([1.7, 64.0, -0.3], [2.3, 65.9, 0.3])],
        ..stone.clone()
    };
    assert_eq!(place(&block, [2, 63, 0], 1, &occupied), LocalUse::Nothing);
    // A replaceable clicked block is replaced in place, whatever the face.
    let grass = surroundings("minecraft:short_grass", "minecraft:stone");
    assert_eq!(place(&block, [2, 64, 0], 4, &grass), LocalUse::Place);
    assert_eq!(place(&block, [0, 64, 0], 4, &grass), LocalUse::Nothing);
    // Interactive blocks succeed unless sneaking with an item.
    let chest = surroundings("minecraft:chest", "minecraft:air");
    assert_eq!(place(&empty, [2, 63, 0], 1, &chest), LocalUse::Interact);
    assert_eq!(place(&block, [2, 63, 0], 1, &chest), LocalUse::Interact);
    let sneaking = UseSurroundings {
        sneaking: true,
        ..chest
    };
    assert_eq!(place(&block, [2, 63, 0], 1, &sneaking), LocalUse::Place);
    assert_eq!(place(&empty, [2, 63, 0], 1, &sneaking), LocalUse::Interact);
    let iron = surroundings("minecraft:iron_door", "minecraft:air");
    assert_eq!(place(&empty, [2, 63, 0], 1, &iron), LocalUse::Nothing);
}

/// Obstruction tests the placed block's own shape, and a replaced block is the destination.
#[test]
fn placement_obstruction_uses_the_placed_shape_and_resolved_cell() {
    let survival = GameModeCapabilities::for_mode(PlayerGameMode::Survival);
    let block = verified(network_item(2, 77));
    // A sneaking player (1.5 tall) standing at y 64 reaches 65.5.
    let around = |placed_boxes| UseSurroundings {
        player_box: ([0.2, 64.0, 0.2], [0.8, 65.5, 0.8]),
        placed_boxes,
        sneaking: true,
        ..surroundings("minecraft:stone", "minecraft:air")
    };
    let place =
        |around: &UseSurroundings| LocalUse::resolve(&block, [0, 66, 0], 0, around, &survival);
    assert_eq!(
        place(&around(None)),
        LocalUse::Nothing,
        "a full cell overlaps"
    );
    let top_slab = vec![([0.0, 0.5, 0.0], [1.0, 1.0, 1.0])];
    assert_eq!(place(&around(Some(top_slab))), LocalUse::Place);
    let bottom_slab = vec![([0.0, 0.0, 0.0], [1.0, 0.5, 1.0])];
    assert_eq!(place(&around(Some(bottom_slab))), LocalUse::Nothing);
    assert_eq!(
        place(&around(Some(Vec::new()))),
        LocalUse::Place,
        "no collision"
    );
    let grass = surroundings("minecraft:short_grass", "minecraft:stone");
    assert_eq!(grass.destination([2, 64, 0], 4), ([2, 64, 0], true));
    let stone = surroundings("minecraft:stone", "minecraft:air");
    assert_eq!(stone.destination([2, 64, 0], 4), ([1, 64, 0], true));
}

/// Only a stateless full cube that does not merge into the clicked block is predicted.
#[test]
fn only_certain_placement_states_are_predicted() {
    let stone = Some("minecraft:stone");
    assert!(placement_state_is_certain(
        true,
        Some("{}"),
        stone,
        Some("minecraft:dirt")
    ));
    assert!(!placement_state_is_certain(
        true,
        Some(r#"{"pillar_axis":"y"}"#),
        Some("minecraft:oak_log"),
        None
    ));
    assert!(!placement_state_is_certain(
        false,
        Some("{}"),
        Some("minecraft:glass_pane"),
        None
    ));
    assert!(!placement_state_is_certain(true, Some("{}"), stone, stone));
    assert!(!placement_state_is_certain(true, None, stone, None));
    assert!(!placement_state_is_certain(true, Some("{}"), None, None));
}

/// Adventure uses doors and containers but cannot place; each ability gates only its own use.
#[test]
fn interaction_and_placement_follow_their_own_abilities() {
    let block = verified(network_item(2, 77));
    let adventure = GameModeCapabilities::for_mode(PlayerGameMode::Adventure);
    let resolve = |clicked: &str, caps: &GameModeCapabilities| {
        LocalUse::resolve(
            &block,
            [2, 63, 0],
            1,
            &surroundings(clicked, "minecraft:air"),
            caps,
        )
    };
    assert_eq!(
        resolve("minecraft:oak_door", &adventure),
        LocalUse::Interact
    );
    assert_eq!(resolve("minecraft:chest", &adventure), LocalUse::Interact);
    assert_eq!(resolve("minecraft:stone", &adventure), LocalUse::Nothing);
    assert!(adventure.can_use_blocks());
    let no_switches = GameModeCapabilities {
        can_use_switches: false,
        ..GameModeCapabilities::for_mode(PlayerGameMode::Survival)
    };
    assert_eq!(
        resolve("minecraft:stone_button", &no_switches),
        LocalUse::Place
    );
    assert_eq!(
        resolve("minecraft:barrel", &no_switches),
        LocalUse::Interact
    );
    let mine_only = GameModeCapabilities {
        can_build: false,
        ..adventure
    };
    assert_eq!(resolve("minecraft:stone", &mine_only), LocalUse::Nothing);
}

#[test]
fn successful_uses_swing_before_their_always_sent_transaction() {
    let observed = crate::interaction_authority::FrozenBlockObservation::fixture(
        [2, 63, 0],
        1,
        verified(network_item(2, 77)),
    );
    let kinds = |local_use| {
        use_packets(
            &observed,
            [0.5, 65.62, 0.5],
            ItemUseTrigger::PlayerInput,
            local_use,
            42,
            |_| true,
            101,
        )
        .iter()
        .map(|packet| format!("{:?}", packet.header.id))
        .collect::<Vec<_>>()
    };
    assert_eq!(
        kinds(LocalUse::Place),
        ["AnimatePacket", "InventoryTransactionPacket"]
    );
    assert_eq!(
        kinds(LocalUse::Interact),
        ["AnimatePacket", "InventoryTransactionPacket"]
    );
    assert_eq!(kinds(LocalUse::Nothing), ["InventoryTransactionPacket"]);
    let guarded = use_packets(
        &observed,
        [0.5, 65.62, 0.5],
        ItemUseTrigger::SimulationTick,
        LocalUse::Place,
        42,
        |_| false,
        101,
    );
    assert_eq!(
        guarded.len(),
        1,
        "the half-swing guard suppresses the animation only"
    );
}

#[test]
fn unknown_or_inventory_pending_selection_fails_closed() {
    let mut ui = UiRuntime::new(7);
    ui.publish_player_game_mode(protocol::PlayerGameMode::Survival);
    ui.inventory_ledger_mut()
        .apply(&InventoryEvent::Authority(InventoryAuthority::Server));
    assert!(ui.inventory_ledger_mut().request_personal_open(42));
    assert!(ui.inventory_ledger_mut().mark_transport_enqueued(0));
    ui.set_local_selected_slot(2);
    assert!(verified_use_selection(&ui).is_none());

    ui.inventory_ledger_mut()
        .apply(&inventory_slot(2, network_item(2, 77)));
    let selection = verified_use_selection(&ui).unwrap();
    assert_eq!(selection.slot, 2);
    assert_eq!(selection.item.block_runtime_id(), 77);

    ui.inventory_ledger_mut()
        .apply(&inventory_slot(3, network_item(3, 0)));
    ui.inventory_ledger_mut().begin_click(3).unwrap();
    assert!(verified_use_selection(&ui).is_none());

    let mut pending_hotbar = UiRuntime::new(7);
    pending_hotbar.publish_player_game_mode(protocol::PlayerGameMode::Survival);
    pending_hotbar
        .inventory_ledger_mut()
        .apply(&inventory_slot(4, NetworkItemStack::empty()));
    pending_hotbar.queue_local_hotbar_selection(4);
    assert!(verified_use_selection(&pending_hotbar).is_none());
}

#[test]
fn a_position_authority_change_drops_the_press_and_schedule() {
    let mut runtime = BlockUseRuntime::default();
    runtime.synchronize((7, 0));
    runtime.latched_press = true;
    runtime.last_use_millis = Some(900);
    runtime.synchronize((7, 1));
    assert_eq!(runtime.due(false, 1, clock(1_000, 0.0)), None);
    assert_eq!(runtime.last_use_millis, None);
}
