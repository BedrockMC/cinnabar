use bytes::Bytes;
use protocol::*;
use sha2::{Digest, Sha256};
use std::sync::Arc;
use valentine::bedrock::version::v1_26_44::McpePacketData;

fn admitted(fixture: &'static [u8]) -> RecipeUpdate {
    let mut batch = Bytes::from_static(fixture);
    let raw = jolyne::batch::decode_batch_raw(&mut batch, false, Some(4096))
        .unwrap()
        .remove(0);
    decode_recipe_update(raw.body()).unwrap()
}
fn registry() -> Vec<ItemRegistryEntry> {
    [(6, "minecraft:oak_log"), (7, "minecraft:oak_planks")]
        .into_iter()
        .map(|(network_id, name)| ItemRegistryEntry {
            identifier: Arc::from(name),
            network_id,
            component_based: false,
            version: ItemRegistryVersion::None,
            component_digest: [0; 32],
            negotiated_max_stack_size: Some(64),
            canonical_empty_component_data: true,
        })
        .collect()
}
fn input(slot: u8, stack_id: i32) -> ManualCraftInput {
    let digest: [u8; 32] = Sha256::digest([]).into();
    let stack = NetworkItemStack {
        network_id: 6,
        metadata: 0,
        count: 1,
        stack_network_id: stack_id,
        nbt_digest: digest,
        block_runtime_id: 0,
        extra_data: Arc::from([]),
    };
    ManualCraftInput {
        slot,
        stack: VerifiedNetworkItemStack::try_new(stack, digest).unwrap(),
    }
}
fn cursor() -> VerifiedNetworkItemStack {
    let stack = NetworkItemStack::empty();
    let digest = stack.nbt_digest;
    VerifiedNetworkItemStack::try_new(stack, digest).unwrap()
}
fn catalog() -> RecipeCatalog {
    let mut catalog = RecipeCatalog::default();
    catalog.begin_session(1);
    catalog.apply(
        1,
        1,
        &admitted(include_bytes!(
            "../fixtures/crafting_data_manual_named_1x1.bin"
        )),
    );
    catalog
}

#[test]
fn candidate_compound_order_and_current_request_reference_match_pinned_codec() {
    let catalog = catalog();
    let plan = ManualCraftPlan::prepare(
        &catalog,
        1,
        17,
        [Some(input(28, 101)), None, None, None],
        &registry(),
    )
    .unwrap();
    let mut packet = manual_craft_packet(&catalog, &plan, -3, &cursor()).unwrap();
    packet.header.from_subclient = 1;
    packet.header.to_subclient = 2;
    assert_eq!(
        encode(&packet, &BedrockSession { shield_item_id: 0 })
            .unwrap()
            .as_ref(),
        include_bytes!("../fixtures/item_stack_request_manual_craft.bin")
    );
    let McpePacketData::ItemStackRequestPacket(request) = packet.data else {
        panic!("request");
    };
    let actions = &request.requests[0].actions;
    assert_eq!(actions.len(), 4);
    use valentine::bedrock::version::v1_26_44::ItemStackRequestPacketDataRequestDataActionsItem as Action;
    assert!(matches!(actions[0], Action::CraftRecipeActionData(_)));
    assert!(matches!(actions[1], Action::CraftResultsActionData(_)));
    assert!(matches!(actions[2], Action::ConsumeActionData(_)));
    let Action::TakeActionData(take) = &actions[3] else {
        panic!("take");
    };
    assert_eq!((take.source.slot, take.source.net_id_variant), (50, -3));
    assert_eq!(
        (take.destination.slot, take.destination.net_id_variant),
        (0, 0)
    );
}

#[test]
fn vertical_named_shape_uses_personal_grid_stride_and_positive_input_ids() {
    let mut catalog = catalog();
    catalog.apply(
        1,
        2,
        &admitted(include_bytes!(
            "../fixtures/crafting_data_manual_named_1x2.bin"
        )),
    );
    let prepare = |a, b| {
        ManualCraftPlan::prepare(
            &catalog,
            1,
            18,
            [Some(input(28, a)), None, Some(input(30, b)), None],
            &registry(),
        )
    };
    let plan = prepare(101, 102).unwrap();
    let packet = manual_craft_packet(&catalog, &plan, -3, &cursor()).unwrap();
    let McpePacketData::ItemStackRequestPacket(packet) = packet.data else {
        panic!("request");
    };
    assert_eq!(packet.requests[0].actions.len(), 5);
    assert!(prepare(-1, 102).is_err());
    assert!(prepare(101, 101).is_err());
    assert!(
        ManualCraftPlan::prepare(
            &catalog,
            1,
            18,
            [Some(input(28, 101)), Some(input(29, 102)), None, None],
            &registry()
        )
        .is_err()
    );
}

#[test]
fn replaced_or_retired_recipe_plan_cannot_emit_and_generic_negative_slots_stay_invalid() {
    let mut catalog = catalog();
    let plan = ManualCraftPlan::prepare(
        &catalog,
        1,
        17,
        [Some(input(28, 101)), None, None, None],
        &registry(),
    )
    .unwrap();
    assert!(manual_craft_packet(&catalog, &plan, -2, &cursor()).is_err());
    catalog.apply(
        1,
        2,
        &admitted(include_bytes!(
            "../fixtures/crafting_data_manual_unsupported_replacement.bin"
        )),
    );
    assert!(catalog.recipe(17).is_none());
    assert_eq!(
        manual_craft_packet(&catalog, &plan, -3, &cursor()).unwrap_err(),
        ManualCraftError::Unavailable
    );
    catalog.apply(
        1,
        3,
        &admitted(include_bytes!(
            "../fixtures/crafting_data_manual_clear_empty.bin"
        )),
    );
    catalog.begin_session(2);
    assert!(manual_craft_packet(&catalog, &plan, -3, &cursor()).is_err());
    assert!(
        item_stack_request_packet(
            -3,
            StackRequestAction::Take {
                amount: 1,
                source: StackRequestSlot {
                    container: StackRequestContainer::PlayerInventory,
                    slot: 0,
                    stack_network_id: -3
                },
                destination: StackRequestSlot {
                    container: StackRequestContainer::Cursor,
                    slot: 0,
                    stack_network_id: 0
                },
            }
        )
        .is_err()
    );
}

#[test]
fn synthetic_acceptance_response_retains_exact_request_and_result_mapping() {
    let packets = decode_batch(
        Bytes::from_static(include_bytes!(
            "../fixtures/item_stack_response_manual_craft.bin"
        )),
        &BedrockSession { shield_item_id: 0 },
    )
    .unwrap();
    let packet = packets.into_iter().next().unwrap();
    let Some(WorldEvent::Inventory(InventoryEvent::Response(response))) =
        into_world_event(packet, 0).unwrap()
    else {
        panic!("response");
    };
    assert_eq!(response.responses.len(), 2);
    let accepted = &response.responses[0];
    assert_eq!(
        (accepted.request_id, accepted.status),
        (-3, StackResponseStatus::Accepted)
    );
    assert_eq!(accepted.containers[0].slots[0].count, 0);
    assert_eq!(
        (
            accepted.containers[1].slots[0].count,
            accepted.containers[1].slots[0].item_stack_id
        ),
        (4, 201)
    );
    assert_eq!(
        (
            response.responses[1].request_id,
            response.responses[1].status
        ),
        (-5, StackResponseStatus::Rejected)
    );
}

#[test]
fn cursor_and_output_binding_must_be_proven_not_guessed() {
    let catalog = catalog();
    let inputs = || [Some(input(28, 101)), None, None, None];
    let plan = ManualCraftPlan::prepare(&catalog, 1, 17, inputs(), &registry()).unwrap();
    assert!(manual_craft_packet(&catalog, &plan, -3, &input(28, 101).stack).is_err());
    let mut entries = registry();
    entries[1].negotiated_max_stack_size = None;
    assert!(ManualCraftPlan::prepare(&catalog, 1, 17, inputs(), &entries).is_err());
    entries[1].negotiated_max_stack_size = Some(3);
    assert!(ManualCraftPlan::prepare(&catalog, 1, 17, inputs(), &entries).is_err());
    entries[1].negotiated_max_stack_size = Some(64);
    entries[1].component_based = true;
    entries[1].canonical_empty_component_data = false;
    assert!(ManualCraftPlan::prepare(&catalog, 1, 17, inputs(), &entries).is_err());
    let duplicate = registry()[1].clone();
    entries = registry();
    entries.push(duplicate);
    assert!(ManualCraftPlan::prepare(&catalog, 1, 17, inputs(), &entries).is_err());
}
