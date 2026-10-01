//! Crafting requests ported from the owner's proxy prediction tests.

use std::sync::Arc;

use protocol::{
    CONTAINER_NAME_CRAFT_INPUT, CONTAINER_NAME_CURSOR, ContainerIdentity, ContainerOpenEvent,
    CraftGridMatch, InventoryContentEvent, InventoryEvent, InventorySlotEvent, ItemRegistryEntry,
    ItemRegistryEvent, ItemRegistryVersion, ItemStackResponseEvent, NetworkItemStack,
    RecipeCatalog, SlotIdentity, StackRequestAction, StackRequestContainer, StackResponse,
    StackResponseContainer, StackResponseSlot, StackResponseStatus, decode_recipe_update,
    match_crafting_grid,
};

use super::*;

const LOG: i32 = 6;
const PLANKS: i32 = 7;
const COBBLE: i32 = 8;
const FURNACE: i32 = 9;

fn varuint(out: &mut Vec<u8>, mut value: u32) {
    while value >= 0x80 {
        out.push(value as u8 | 0x80);
        value >>= 7;
    }
    out.push(value as u8);
}

fn varint(out: &mut Vec<u8>, value: i32) {
    varuint(out, ((value << 1) ^ (value >> 31)) as u32);
}

fn string(out: &mut Vec<u8>, value: &str) {
    varuint(out, value.len() as u32);
    out.extend_from_slice(value.as_bytes());
}

/// One crafting-table record in the family's wire grammar.
fn record(
    out: &mut Vec<u8>,
    shape: Option<(i32, i32)>,
    cells: &[Option<&str>],
    output: i32,
    id: u32,
) {
    string(out, "test:recipe");
    if let Some((width, height)) = shape {
        varint(out, width);
        varint(out, height);
    }
    varuint(out, cells.len() as u32);
    for cell in cells {
        match cell {
            Some(name) => {
                varuint(out, 1);
                string(out, "name");
                string(out, name);
                varint(out, 0);
                varint(out, 1);
            }
            None => {
                varuint(out, 0);
                varint(out, 0);
                varint(out, 0);
            }
        }
    }
    varuint(out, 1);
    varint(out, output);
    out.extend_from_slice(&1u16.to_le_bytes());
    varuint(out, 0);
    varint(out, 0);
    varuint(out, 0);
    out.extend_from_slice(&[0; 16]);
    string(out, "crafting_table");
    varint(out, 0);
    if shape.is_some() {
        out.push(0);
    }
    out.push(0);
    varuint(out, id);
}

fn recipes() -> InventoryEvent {
    let mut body = Vec::new();
    varuint(&mut body, 2);
    record(
        &mut body,
        Some((1, 1)),
        &[Some("minecraft:oak_log")],
        PLANKS,
        1,
    );
    let ring: Vec<Option<&str>> = (0..9)
        .map(|index| (index != 4).then_some("minecraft:cobblestone"))
        .collect();
    record(&mut body, Some((3, 3)), &ring, FURNACE, 2);
    varuint(&mut body, 1);
    record(
        &mut body,
        None,
        &[Some("minecraft:oak_log"), Some("minecraft:cobblestone")],
        FURNACE,
        3,
    );
    for _ in 2..11 {
        varuint(&mut body, 0);
    }
    body.push(1);
    InventoryEvent::Recipes(decode_recipe_update(&body).unwrap())
}

fn catalog() -> RecipeCatalog {
    let InventoryEvent::Recipes(update) = recipes() else {
        unreachable!()
    };
    let mut catalog = RecipeCatalog::default();
    catalog.begin_session(1);
    assert!(catalog.apply(1, 1, &update));
    catalog
}

fn registry() -> ItemRegistryEvent {
    let entry = |network_id, identifier: &str| ItemRegistryEntry {
        identifier: Arc::from(identifier),
        network_id,
        component_based: false,
        version: ItemRegistryVersion::None,
        component_digest: [0; 32],
        negotiated_max_stack_size: Some(64),
        canonical_empty_component_data: true,
        item_tags: std::sync::Arc::from([]),
    };
    ItemRegistryEvent {
        entries: vec![
            entry(LOG, "minecraft:oak_log"),
            entry(PLANKS, "minecraft:oak_planks"),
            entry(COBBLE, "minecraft:cobblestone"),
            entry(FURNACE, "minecraft:furnace"),
        ]
        .into(),
    }
}

fn stack(network_id: i32, stack_network_id: i32, count: u16) -> NetworkItemStack {
    NetworkItemStack {
        network_id,
        stack_network_id,
        count,
        ..NetworkItemStack::default()
    }
}

fn craft_slot(slot: u16, stack: NetworkItemStack) -> InventoryEvent {
    InventoryEvent::Slot(InventorySlotEvent {
        identity: SlotIdentity {
            container: ContainerIdentity {
                window_id: Some(124),
                slot_type: Some(CONTAINER_NAME_CRAFT_INPUT),
                dynamic_id: None,
            },
            slot,
        },
        stack,
        storage_item: None,
    })
}

fn ledger(window_type: i8) -> PlayerInventoryLedger {
    let mut ledger = PlayerInventoryLedger::default();
    ledger.begin_session(1);
    ledger.apply(&InventoryEvent::Authority(InventoryAuthority::Server));
    ledger.apply_registry(&registry());
    ledger.apply(&InventoryEvent::Content(InventoryContentEvent {
        container: ContainerIdentity::window(0),
        slots: Arc::from(vec![
            NetworkItemStack::default();
            PLAYER_INVENTORY_SLOT_COUNT
        ]),
        storage_item: NetworkItemStack::default(),
    }));
    if window_type == WORKBENCH_WINDOW_TYPE {
        ledger.apply(&InventoryEvent::Open(ContainerOpenEvent {
            container: ContainerIdentity::window(3),
            window_type,
            position: [0, 64, 0],
            runtime_entity_id: -1,
        }));
    } else {
        assert!(ledger.request_personal_open(42));
        assert!(ledger.mark_transport_enqueued(0));
        ledger.apply(&InventoryEvent::Open(ContainerOpenEvent {
            container: ContainerIdentity::window(2),
            window_type,
            position: [0, 64, 0],
            runtime_entity_id: -1,
        }));
    }
    ledger
}

fn unique(ledger: &PlayerInventoryLedger, catalog: &RecipeCatalog) -> protocol::RecipeHandle {
    let cells = ledger.crafting_grid_cells().unwrap();
    let items: Vec<_> = cells
        .iter()
        .map(|cell| cell.as_ref().map(CraftGridCell::item))
        .collect();
    match match_crafting_grid(catalog, ledger.crafting_grid().width(), &items) {
        CraftGridMatch::Unique(recipe) => recipe,
        other => panic!("expected one recipe, got {other:?}"),
    }
}

fn respond(ledger: &mut PlayerInventoryLedger, request_id: i32, rows: &[(u8, u8, u8, i32)]) {
    let containers: Vec<_> = rows
        .iter()
        .map(|(name, slot, count, id)| StackResponseContainer {
            container: ContainerIdentity {
                window_id: None,
                slot_type: Some(*name),
                dynamic_id: None,
            },
            slots: Arc::from([StackResponseSlot {
                slot: *slot,
                hotbar_slot: *slot,
                count: *count,
                item_stack_id: *id,
                custom_name: Arc::from(""),
                filtered_custom_name: Arc::from(""),
                durability_correction: 0,
            }]),
        })
        .collect();
    ledger.apply(&InventoryEvent::Response(ItemStackResponseEvent {
        responses: Arc::from([StackResponse {
            status: StackResponseStatus::Accepted,
            request_id,
            containers: Arc::from(containers),
        }]),
    }));
}

/// The personal flow sends CraftRecipe, CraftResultsDeprecated, Consume and a
/// Take from created output named by the request id, and settles cleanly.
#[test]
fn personal_craft_consumes_inputs_and_takes_output_to_the_cursor() {
    let catalog = catalog();
    let mut ledger = ledger(PERSONAL_INVENTORY_WINDOW_TYPE);
    ledger.apply(&craft_slot(29, stack(LOG, 101, 3)));
    let recipe = unique(&ledger, &catalog);
    let request = ledger.begin_craft(&recipe, 1).unwrap();
    let actions = &ledger.newest_request().unwrap().actions;
    assert!(matches!(
        actions[0],
        StackRequestAction::CraftRecipe {
            recipe_network_id: 1,
            crafts: 1
        }
    ));
    assert!(matches!(
        actions[1],
        StackRequestAction::CraftResultsDeprecated { crafts: 1, .. }
    ));
    let StackRequestAction::Consume { amount, source } = actions[2] else {
        panic!("consume");
    };
    assert_eq!((amount, source.slot, source.stack_network_id), (1, 29, 101));
    assert_eq!(source.container, StackRequestContainer::CraftingInput);
    let StackRequestAction::Take {
        amount,
        source,
        destination,
    } = actions[3]
    else {
        panic!("take");
    };
    assert_eq!(
        (amount, source.slot, source.stack_network_id),
        (1, 50, request)
    );
    assert_eq!(destination.container, StackRequestContainer::Cursor);
    assert!(ledger.pending_batch().unwrap().is_some());

    assert_eq!(
        ledger
            .target_stack(InventoryTarget::Craft(29))
            .unwrap()
            .count,
        2
    );
    let held = ledger.cursor_stack().unwrap();
    assert_eq!((held.network_id, held.stack_network_id), (PLANKS, request));
    assert!(ledger.created_output_stack().is_none());
    assert_eq!(
        ledger.begin_click(0),
        Err(InventoryGestureError::AwaitingIdentity),
        "crafted output cannot be named before it settles"
    );

    assert!(ledger.mark_transport_enqueued(10));
    respond(
        &mut ledger,
        request,
        &[
            (CONTAINER_NAME_CRAFT_INPUT, 29, 2, 101),
            (CONTAINER_NAME_CURSOR, 0, 1, 555),
        ],
    );
    assert_eq!(ledger.cursor_stack().unwrap().stack_network_id, 555);
    assert!(!ledger.resync_required());
}

/// An acceptance that never names the crafted stack leaves it unusable.
#[test]
fn crafted_output_without_a_server_id_requires_recovery() {
    let catalog = catalog();
    let mut ledger = ledger(PERSONAL_INVENTORY_WINDOW_TYPE);
    ledger.apply(&craft_slot(28, stack(LOG, 101, 1)));
    let request = ledger.begin_craft(&unique(&ledger, &catalog), 1).unwrap();
    assert!(ledger.mark_transport_enqueued(10));
    respond(&mut ledger, request, &[]);
    assert!(ledger.resync_required());
}

/// A partial match predicts nothing and allocates no request.
#[test]
fn unmatched_ingredients_predict_nothing() {
    let catalog = catalog();
    let mut ledger = ledger(PERSONAL_INVENTORY_WINDOW_TYPE);
    ledger.apply(&craft_slot(28, stack(LOG, 101, 1)));
    ledger.apply(&craft_slot(29, stack(COBBLE, 102, 1)));
    let shapeless = unique(&ledger, &catalog);
    assert_eq!(shapeless.network_id(), 3);
    ledger.apply(&craft_slot(29, NetworkItemStack::default()));
    assert_eq!(
        ledger.begin_craft(&shapeless, 1),
        Err(InventoryGestureError::InvalidRequest)
    );
    assert_eq!(ledger.pending_request_count(), 0);
    assert_eq!(ledger.next_request_id, -3);

    ledger.apply(&craft_slot(29, stack(COBBLE, 102, 1)));
    ledger.apply(&cursor_content(stack(COBBLE, 103, 1)));
    assert_eq!(
        ledger.begin_craft(&shapeless, 1),
        Err(InventoryGestureError::InvalidRequest),
        "an occupied cursor cannot take the output"
    );
}

/// The workbench grid is UI slots 32..=40 and forms 3x3 recipes.
#[test]
fn workbench_crafts_three_by_three_from_its_grid() {
    let catalog = catalog();
    let mut ledger = ledger(WORKBENCH_WINDOW_TYPE);
    assert_eq!(ledger.crafting_grid(), CraftingGrid::Workbench);
    for (index, slot) in (32..41).enumerate() {
        if index != 4 {
            ledger.apply(&craft_slot(slot, stack(COBBLE, 200 + slot as i32, 1)));
        }
    }
    let recipe = unique(&ledger, &catalog);
    assert_eq!(recipe.network_id(), 2);
    let request = ledger.begin_craft(&recipe, 1).unwrap();
    let consumed: Vec<u8> = ledger
        .newest_request()
        .unwrap()
        .actions
        .iter()
        .filter_map(|action| match action {
            StackRequestAction::Consume { source, .. } => Some(source.slot),
            _ => None,
        })
        .collect();
    assert_eq!(consumed, [32, 33, 34, 35, 37, 38, 39, 40]);
    assert_eq!(ledger.cursor_stack().unwrap().stack_network_id, request);
    for slot in 32..41 {
        assert!(ledger.target_stack(InventoryTarget::Craft(slot)).is_none());
    }
}

fn cursor_content(stack: NetworkItemStack) -> InventoryEvent {
    InventoryEvent::Content(InventoryContentEvent {
        container: ContainerIdentity {
            window_id: Some(-1),
            slot_type: Some(CONTAINER_NAME_CURSOR),
            dynamic_id: None,
        },
        slots: Arc::from([stack]),
        storage_item: NetworkItemStack::default(),
    })
}

fn creative(ledger: &mut PlayerInventoryLedger) {
    ledger.apply(&InventoryEvent::Creative(protocol::CreativeContentEvent {
        groups: Arc::from([]),
        items: Arc::from([protocol::CreativeItem {
            creative_network_id: 44,
            stack: stack(COBBLE, -1, 1),
            group: 0,
        }]),
        skipped: 0,
    }));
}

/// A creative take crafts the entry and transfers a full stack out of created
/// output, named by the request id.
#[test]
fn creative_take_moves_a_full_stack_into_the_cursor() {
    let mut ledger = ledger(PERSONAL_INVENTORY_WINDOW_TYPE);
    creative(&mut ledger);
    let request = ledger
        .begin_creative_take(44, CreativeDestination::Cursor)
        .unwrap();
    let actions = &ledger.newest_request().unwrap().actions;
    assert!(matches!(
        actions[0],
        StackRequestAction::CraftCreative {
            creative_item_network_id: 44,
            crafts: 1
        }
    ));
    assert!(matches!(
        &actions[1],
        StackRequestAction::CraftResultsDeprecated { results, crafts: 1 } if results.is_empty()
    ));
    let StackRequestAction::Take {
        amount,
        source,
        destination,
    } = actions[2]
    else {
        panic!("take into the cursor");
    };
    assert_eq!(
        (amount, source.slot, source.stack_network_id),
        (64, 50, request)
    );
    assert_eq!(destination.container, StackRequestContainer::Cursor);
    assert!(ledger.pending_batch().unwrap().is_some());
    let held = ledger.cursor_stack().unwrap();
    assert_eq!((held.network_id, held.count), (COBBLE, 64));

    assert!(ledger.mark_transport_enqueued(10));
    respond(&mut ledger, request, &[(CONTAINER_NAME_CURSOR, 0, 64, 700)]);
    assert_eq!(ledger.cursor_stack().unwrap().stack_network_id, 700);
    assert!(!ledger.resync_required());
}

/// Unknown entries and occupied destinations send nothing.
#[test]
fn creative_take_refuses_unknown_entries_and_occupied_destinations() {
    let mut ledger = ledger(PERSONAL_INVENTORY_WINDOW_TYPE);
    assert_eq!(
        ledger.begin_creative_take(44, CreativeDestination::Cursor),
        Err(InventoryGestureError::InvalidRequest)
    );
    creative(&mut ledger);
    assert_eq!(
        ledger.begin_creative_take(45, CreativeDestination::Cursor),
        Err(InventoryGestureError::InvalidRequest)
    );
    ledger.apply(&InventoryEvent::Slot(InventorySlotEvent {
        identity: SlotIdentity {
            container: ContainerIdentity::window(0),
            slot: 3,
        },
        stack: stack(LOG, 90, 1),
        storage_item: None,
    }));
    assert_eq!(
        ledger.begin_creative_take(44, CreativeDestination::Player(3)),
        Err(InventoryGestureError::InvalidRequest)
    );
    assert_eq!(ledger.pending_request_count(), 0);
    let request = ledger
        .begin_creative_take(44, CreativeDestination::Player(4))
        .unwrap();
    assert!(matches!(
        ledger.newest_request().unwrap().actions[2],
        StackRequestAction::Place { amount: 64, .. }
    ));
    assert_eq!(ledger.displayed_stack(4).unwrap().stack_network_id, request);
}

/// Closing either crafting screen returns the grid server-side, so no ghost
/// cells or stale ids survive into the next screen.
#[test]
fn closing_a_crafting_screen_clears_the_grid() {
    let mut personal = ledger(PERSONAL_INVENTORY_WINDOW_TYPE);
    personal.apply(&craft_slot(28, stack(LOG, 101, 1)));
    personal.request_personal_close();
    assert!(personal.mark_transport_enqueued(10));
    personal.apply(&InventoryEvent::Close(protocol::ContainerCloseEvent {
        container: ContainerIdentity::window(2),
        window_type: PERSONAL_INVENTORY_WINDOW_TYPE,
        server_initiated: false,
    }));
    assert!(personal.target_stack(InventoryTarget::Craft(28)).is_none());
    assert!(
        personal
            .crafting_grid_cells()
            .unwrap()
            .iter()
            .all(Option::is_none)
    );

    let mut workbench = ledger(WORKBENCH_WINDOW_TYPE);
    workbench.apply(&craft_slot(36, stack(COBBLE, 201, 1)));
    workbench.request_storage_close();
    assert!(workbench.target_stack(InventoryTarget::Craft(36)).is_none());
    assert_eq!(workbench.crafting_grid(), CraftingGrid::Personal);
}

// The recipe book's filter shows recipes the inventory holds some ingredient of.
#[test]
fn a_held_ingredient_marks_uncraftable_recipes_as_partly_held() {
    let catalog = catalog();
    let mut ledger = ledger(PERSONAL_INVENTORY_WINDOW_TYPE);
    let mut slots = vec![NetworkItemStack::default(); PLAYER_INVENTORY_SLOT_COUNT];
    slots[9] = stack(LOG, 101, 1);
    ledger.apply(&InventoryEvent::Content(InventoryContentEvent {
        container: ContainerIdentity::window(0),
        slots: Arc::from(slots),
        storage_item: NetworkItemStack::default(),
    }));
    let by_id = |id: u32| {
        catalog
            .crafting_handles()
            .into_iter()
            .find(|recipe| recipe.network_id() == id)
            .unwrap()
    };
    let (planks, ring, log_and_cobble) = (by_id(1), by_id(2), by_id(3));
    assert!(ledger.can_auto_craft(&planks));
    assert!(!ledger.can_auto_craft(&log_and_cobble));
    assert!(ledger.holds_any_ingredient(&log_and_cobble));
    assert!(!ledger.holds_any_ingredient(&ring));
}
