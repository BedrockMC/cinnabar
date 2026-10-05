use std::sync::Arc;

use protocol::{
    ContainerIdentity, ContainerOpenEvent, InventoryAuthority, InventoryContentEvent,
    InventoryEvent, NetworkItemStack,
};

use crate::ui_runtime::inventory_ledger::{
    INVENTORY_REQUEST_TIMEOUT_MILLIS, InventoryPendingState, PERSONAL_INVENTORY_WINDOW_TYPE,
    PlayerInventoryLedger,
};
use crate::ui_runtime::{UiRuntime, flush_inventory_send};

fn stack(network_id: i32, count: u16, stack_network_id: i32) -> NetworkItemStack {
    NetworkItemStack {
        network_id,
        metadata: 0,
        count,
        stack_network_id,
        block_runtime_id: 0,
        extra_data: Arc::from([]),
        nbt_digest: [0; 32],
    }
}

fn open_personal_inventory(ledger: &mut PlayerInventoryLedger) {
    assert!(ledger.request_personal_open(42));
    assert!(ledger.mark_transport_enqueued(0));
    ledger.apply(&InventoryEvent::Open(ContainerOpenEvent {
        container: ContainerIdentity::window(2),
        window_type: PERSONAL_INVENTORY_WINDOW_TYPE,
        position: [0, 64, 0],
        runtime_entity_id: -1,
    }));
}

#[test]
fn bounded_transport_pressure_does_not_consume_or_duplicate_the_request() {
    let mut player_runtime = player_state::PlayerState::new(1);

    let mut runtime = UiRuntime::new(1);
    runtime
        .inventory_ledger_mut(&mut player_runtime)
        .apply(&InventoryEvent::Authority(InventoryAuthority::Server));
    open_personal_inventory(runtime.inventory_ledger_mut(&mut player_runtime));
    let content = InventoryEvent::Content(InventoryContentEvent {
        container: ContainerIdentity::window(0),
        slots: Arc::from(
            (0..36)
                .map(|index| {
                    if index == 0 {
                        stack(5, 1, 44)
                    } else {
                        NetworkItemStack::default()
                    }
                })
                .collect::<Vec<_>>(),
        ),
        storage_item: NetworkItemStack::default(),
    });
    runtime
        .inventory_ledger_mut(&mut player_runtime)
        .apply(&content);
    let request = runtime
        .inventory_ledger_mut(&mut player_runtime)
        .begin_click(0)
        .unwrap();

    assert_eq!(
        flush_inventory_send(&mut player_runtime, &mut runtime, 10, |_| Err("full")),
        Err("full")
    );
    assert_eq!(
        runtime
            .inventory_ledger(&player_runtime)
            .pending_request_id(),
        Some(request)
    );
    assert_eq!(
        runtime.inventory_ledger(&player_runtime).pending_state(),
        Some(InventoryPendingState::AwaitingTransport)
    );
    assert_eq!(
        flush_inventory_send(
            &mut player_runtime,
            &mut runtime,
            10 + INVENTORY_REQUEST_TIMEOUT_MILLIS,
            |_| Err("full")
        ),
        Err("full")
    );
    assert!(!runtime.inventory_ledger(&player_runtime).resync_required());

    runtime
        .inventory_ledger_mut(&mut player_runtime)
        .apply(&content);
    let retry = runtime
        .inventory_ledger_mut(&mut player_runtime)
        .begin_click(0)
        .unwrap();
    assert_eq!(
        flush_inventory_send(&mut player_runtime, &mut runtime, 11, |_| Ok::<_, &str>(())),
        Ok(true)
    );
    assert_eq!(
        flush_inventory_send(&mut player_runtime, &mut runtime, 12, |_| Ok::<_, &str>(())),
        Ok(false)
    );
    assert_ne!(request, retry);
}
