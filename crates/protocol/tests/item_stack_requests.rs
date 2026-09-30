use protocol::{
    BedrockSession, InventoryPacketError, MAX_STACK_REQUEST_ACTIONS, StackRequestAction,
    StackRequestContainer, StackRequestSlot, container_close_packet, encode,
    item_stack_request_packet,
};

fn slot(container: StackRequestContainer, slot: u8, stack_network_id: i32) -> StackRequestSlot {
    StackRequestSlot {
        container,
        slot,
        stack_network_id,
    }
}

fn body(action: StackRequestAction) -> Vec<u8> {
    request_body(-3, action)
}

fn request_body(request_id: i32, action: StackRequestAction) -> Vec<u8> {
    encode(
        &item_stack_request_packet(request_id, &[action]).expect("valid request"),
        &BedrockSession { shield_item_id: 0 },
    )
    .expect("encode request")
    .to_vec()
}

#[test]
fn personal_take_and_place_encode_empty_destinations_as_stack_id_zero() {
    assert_eq!(
        request_body(
            -3,
            StackRequestAction::Take {
                amount: 32,
                source: slot(StackRequestContainer::PlayerInventory, 0, 9),
                destination: slot(StackRequestContainer::Cursor, 0, 0),
            },
        ),
        hex("fe1b93010105010000201c0000090000003b00000000000000ffffffff")
    );
    assert_eq!(
        request_body(
            -5,
            StackRequestAction::Place {
                amount: 32,
                source: slot(StackRequestContainer::Cursor, 0, 9),
                destination: slot(StackRequestContainer::PlayerInventory, 9, 0),
            },
        ),
        hex("fe1b93010109010101203b0000090000001d00090000000000ffffffff")
    );
}

fn hex(value: &str) -> Vec<u8> {
    value
        .as_bytes()
        .chunks_exact(2)
        .map(|pair| {
            let text = std::str::from_utf8(pair).unwrap();
            u8::from_str_radix(text, 16).unwrap()
        })
        .collect()
}

#[test]
fn take_place_and_swap_have_exact_protocol_2168_wire() {
    let player = slot(StackRequestContainer::PlayerInventory, 4, 91);
    let cursor = slot(StackRequestContainer::Cursor, 0, -1);

    assert_eq!(
        body(StackRequestAction::Take {
            amount: 3,
            source: player,
            destination: cursor,
        }),
        hex("fe1b93010105010000031c00045b0000003b0000ffffffff00ffffffff")
    );
    assert_eq!(
        body(StackRequestAction::Place {
            amount: 3,
            source: cursor,
            destination: player,
        }),
        hex("fe1b93010105010101033b0000ffffffff1c00045b00000000ffffffff")
    );
    assert_eq!(
        body(StackRequestAction::Swap {
            source: player,
            destination: cursor,
        }),
        hex("fe1a930101050102021c00045b0000003b0000ffffffff00ffffffff")
    );
}

#[test]
fn level_entity_take_and_client_close_match_captured_protocol_2168_wire() {
    let storage = slot(
        StackRequestContainer::LevelEntity { dynamic_id: None },
        2,
        91,
    );
    let cursor = slot(StackRequestContainer::Cursor, 0, -1);
    assert_eq!(
        body(StackRequestAction::Take {
            amount: 3,
            source: storage,
            destination: cursor,
        }),
        hex("fe1b93010105010000030700025b0000003b0000ffffffff00ffffffff")
    );
    assert_eq!(
        encode(
            &container_close_packet(1, 0).expect("valid close"),
            &BedrockSession { shield_item_id: 0 },
        )
        .unwrap()
        .to_vec(),
        hex("fe042f010000")
    );
    assert_eq!(
        encode(
            &container_close_packet(-1, 0).expect("signed raw close"),
            &BedrockSession { shield_item_id: 0 },
        )
        .unwrap()
        .to_vec(),
        hex("fe042fff0000")
    );
    assert_eq!(
        encode(
            &container_close_packet(-128, 0).expect("lowest signed raw close"),
            &BedrockSession { shield_item_id: 0 },
        )
        .unwrap()
        .to_vec(),
        hex("fe042f800000")
    );
}

#[test]
fn builder_rejects_ids_amounts_counts_and_slots() {
    let player = slot(StackRequestContainer::PlayerInventory, 0, 1);
    let cursor = slot(StackRequestContainer::Cursor, 0, -1);
    let swap = StackRequestAction::Swap {
        source: player,
        destination: cursor,
    };
    assert_eq!(
        item_stack_request_packet(0, std::slice::from_ref(&swap)).unwrap_err(),
        InventoryPacketError::InvalidStackRequestId
    );
    assert_eq!(
        item_stack_request_packet(
            -3,
            &[StackRequestAction::Take {
                amount: 0,
                source: player,
                destination: cursor,
            }],
        )
        .unwrap_err(),
        InventoryPacketError::InvalidStackRequestAmount
    );
    assert!(
        item_stack_request_packet(
            -3,
            &[StackRequestAction::Swap {
                source: slot(StackRequestContainer::PlayerInventory, 36, 1),
                destination: cursor,
            }],
        )
        .is_err()
    );
    assert_eq!(
        item_stack_request_packet(-3, &[]).unwrap_err(),
        InventoryPacketError::InvalidStackRequestActionCount(0)
    );
    let too_many = vec![swap.clone(); MAX_STACK_REQUEST_ACTIONS + 1];
    assert_eq!(
        item_stack_request_packet(-3, &too_many).unwrap_err(),
        InventoryPacketError::InvalidStackRequestActionCount(MAX_STACK_REQUEST_ACTIONS + 1)
    );
    assert!(item_stack_request_packet(-3, &too_many[1..]).is_ok());
}

/// Created output is named by this request's own id; any other negative id
/// is refused.
#[test]
fn negative_stack_ids_may_only_name_this_requests_output() {
    let take = |id| StackRequestAction::Take {
        amount: 1,
        source: slot(StackRequestContainer::CreatedOutput, 50, id),
        destination: slot(StackRequestContainer::Cursor, 0, 0),
    };
    assert!(item_stack_request_packet(-7, &[take(-7)]).is_ok());
    assert_eq!(
        item_stack_request_packet(-7, &[take(-5)]).unwrap_err(),
        InventoryPacketError::InvalidRequestStackNetworkId(-5)
    );
    let from_player = StackRequestAction::Take {
        amount: 1,
        source: slot(StackRequestContainer::PlayerInventory, 0, -7),
        destination: slot(StackRequestContainer::Cursor, 0, 0),
    };
    assert_eq!(
        item_stack_request_packet(-7, &[from_player]).unwrap_err(),
        InventoryPacketError::InvalidRequestStackNetworkId(-7)
    );
}

/// Offhand cells always go out as wire slot 1; players 0..9 use the hotbar
/// name and 9..36 the inventory name. The bytes match the owner's captured
/// quick transfer from inventory slot 14 into the offhand.
#[test]
fn fixed_windows_use_vanilla_names_and_wire_slots() {
    let place = |destination| {
        body(StackRequestAction::Place {
            amount: 1,
            source: slot(StackRequestContainer::PlayerInventory, 14, 493),
            destination,
        })
    };
    let offhand_zero = place(slot(StackRequestContainer::Offhand, 0, 0));
    let offhand_one = place(slot(StackRequestContainer::Offhand, 1, 0));
    assert_eq!(offhand_zero, offhand_one);
    assert_eq!(
        offhand_one,
        hex("fe1b93010105010101011d000eed0100002200010000000000ffffffff")
    );
    for (container, slot_index) in [
        (StackRequestContainer::Armor, 5),
        (StackRequestContainer::Offhand, 2),
        (StackRequestContainer::CraftingInput, 27),
        (StackRequestContainer::CraftingInput, 41),
        (StackRequestContainer::CreatedOutput, 0),
        (
            StackRequestContainer::OpenWindow {
                name: 29,
                dynamic_id: None,
            },
            0,
        ),
    ] {
        assert!(
            item_stack_request_packet(
                -3,
                &[StackRequestAction::Destroy {
                    amount: 1,
                    source: slot(container, slot_index, 5),
                }],
            )
            .is_err(),
            "{container:?} slot {slot_index}"
        );
    }
}
