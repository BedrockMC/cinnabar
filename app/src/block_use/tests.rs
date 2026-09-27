use std::sync::Arc;

use protocol::{
    ContainerIdentity, InventoryAuthority, InventoryEvent, InventorySlotEvent, ItemUseTrigger,
    NetworkItemStack, SlotIdentity, VerifiedNetworkItemStack,
};
use sha2::{Digest, Sha256};

use super::{
    BlockUseRuntime, placement_cell, predict_placement, repeat_interval_millis,
    verified_use_selection,
};
use crate::ui_runtime::UiRuntime;

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
    assert_eq!(repeat_interval_millis(true, 5.0, true), 300);
    assert_eq!(repeat_interval_millis(false, 0.0, true), 200);
    assert_eq!(repeat_interval_millis(false, f32::NAN, false), 200);
    assert_eq!(repeat_interval_millis(false, 4.3, true), 180);
    assert_eq!(repeat_interval_millis(false, 10.0, false), 90);
    assert_eq!(repeat_interval_millis(false, 10.0, true), 100);
}

#[test]
fn a_press_fires_immediately_and_holds_wait_for_their_repeat() {
    let mut runtime = BlockUseRuntime::default();
    assert_eq!(runtime.due(true, 0), None);
    runtime.latched_press = true;
    assert_eq!(runtime.due(false, 0), Some(ItemUseTrigger::PlayerInput));
    runtime.latched_press = false;
    runtime.next_repeat_millis = Some(200);
    assert_eq!(runtime.due(true, 199), None);
    assert_eq!(runtime.due(true, 200), Some(ItemUseTrigger::SimulationTick));
    assert_eq!(runtime.due(false, 200), None);
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

#[test]
fn only_a_block_item_into_clear_air_predicts_success() {
    let block = verified(network_item(2, 77));
    let feet = [0.5, 64.0, 0.5];
    assert!(predict_placement(&block, [2, 64, 0], true, feet));
    assert!(!predict_placement(&block, [2, 64, 0], false, feet));
    // The player's own column cannot receive a block.
    assert!(!predict_placement(&block, [0, 65, 0], true, feet));
    assert!(predict_placement(&block, [0, 66, 0], true, feet));
    assert!(!predict_placement(
        &verified(network_item(3, 0)),
        [2, 64, 0],
        true,
        feet
    ));
    assert!(!predict_placement(
        &verified(NetworkItemStack::empty()),
        [2, 64, 0],
        true,
        feet
    ));
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
