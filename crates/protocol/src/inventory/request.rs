use valentine::bedrock::version::v1_26_51::{
    ActorRuntimeId, ContainerClosePacket, EnumsInteractPacketPayloadAction as InteractAction,
    EnumsTextProcessingEventOrigin, InteractPacket, ItemStackRequestPacket,
    ItemStackRequestPacketDataRequestData, TypedClientNetIdstructItemStackRequestIdTagint32T0,
};

use super::InventoryPacketError;
mod actions;
pub(super) mod manual_craft;
pub(super) mod mining;

pub use actions::{
    ARMOR_SLOTS, AutoCraftIngredient, CRAFTING_INPUT_SLOTS, CREATED_OUTPUT_SLOT, CraftResult,
    PLAYER_INVENTORY_SLOTS, StackItemDescriptor, StackRequestAction, StackRequestContainer,
    StackRequestSlot,
};

/// Actions one request may carry.
pub const MAX_STACK_REQUEST_ACTIONS: usize = 100;

pub fn open_inventory_packet(
    target_runtime_id: u64,
) -> Result<crate::Packet, InventoryPacketError> {
    if target_runtime_id == 0 {
        return Err(InventoryPacketError::InvalidInventoryTargetRuntimeId);
    }
    Ok(InteractPacket {
        action: InteractAction::Openinventory,
        target_runtime_id: ActorRuntimeId {
            actor_runtime_id: target_runtime_id,
        },
        position: None,
    }
    .into())
}

/// Builds one request. A negative stack id may only name this request's own
/// created output, which vanilla identifies by the request id.
pub fn item_stack_request_packet(
    request_id: i32,
    actions: &[StackRequestAction],
) -> Result<crate::Packet, InventoryPacketError> {
    item_stack_request_packet_filtered(request_id, actions, &[])
}

/// Text an anvil rename carries for server-side filtering.
pub const MAX_FILTER_STRINGS: usize = 4;

/// Like [`item_stack_request_packet`], with the anvil-text strings a
/// craft-optional action's filter index points into.
pub fn item_stack_request_packet_filtered(
    request_id: i32,
    actions: &[StackRequestAction],
    filter_strings: &[String],
) -> Result<crate::Packet, InventoryPacketError> {
    if filter_strings.len() > MAX_FILTER_STRINGS {
        return Err(InventoryPacketError::InvalidStackRequestActionCount(
            filter_strings.len(),
        ));
    }
    if request_id >= -1 || request_id & 1 == 0 {
        return Err(InventoryPacketError::InvalidStackRequestId);
    }
    if actions.is_empty() || actions.len() > MAX_STACK_REQUEST_ACTIONS {
        return Err(InventoryPacketError::InvalidStackRequestActionCount(
            actions.len(),
        ));
    }
    for slot in actions.iter().flat_map(action_slots) {
        let created_output = slot.container == StackRequestContainer::CreatedOutput
            && slot.stack_network_id == request_id;
        if slot.stack_network_id < -1 && !created_output {
            return Err(InventoryPacketError::InvalidRequestStackNetworkId(
                slot.stack_network_id,
            ));
        }
    }
    Ok(ItemStackRequestPacket {
        requests: vec![ItemStackRequestPacketDataRequestData {
            client_request_id: TypedClientNetIdstructItemStackRequestIdTagint32T0 {
                id: request_id,
            },
            actions: actions
                .iter()
                .map(actions::encode)
                .collect::<Result<_, _>>()?,
            strings_to_filter: filter_strings.to_vec(),
            strings_to_filter_origin: if filter_strings.is_empty() {
                EnumsTextProcessingEventOrigin::Unknown
            } else {
                EnumsTextProcessingEventOrigin::Anviltext
            },
        }],
    }
    .into())
}

fn action_slots(action: &StackRequestAction) -> impl Iterator<Item = StackRequestSlot> {
    let (first, second) = match action {
        StackRequestAction::Take {
            source,
            destination,
            ..
        }
        | StackRequestAction::Place {
            source,
            destination,
            ..
        }
        | StackRequestAction::Swap {
            source,
            destination,
        } => (Some(*source), Some(*destination)),
        StackRequestAction::Drop { source, .. }
        | StackRequestAction::Destroy { source, .. }
        | StackRequestAction::Consume { source, .. } => (Some(*source), None),
        _ => (None, None),
    };
    first.into_iter().chain(second)
}

/// Closes the named window using the native client's unspecified container type.
pub fn container_close_packet(window_id: i32) -> Result<crate::Packet, InventoryPacketError> {
    let container_id = match window_id {
        -128..=-1 => (window_id as i8).to_ne_bytes()[0],
        0..=255 => window_id as u8,
        _ => {
            return Err(InventoryPacketError::InvalidContainerCloseWindowId(
                window_id,
            ));
        }
    };
    Ok(ContainerClosePacket {
        container_id,
        container_type: (-9_i8).to_ne_bytes()[0],
        server_initiated_close: false,
    }
    .into())
}

#[cfg(test)]
mod tests {
    use valentine::bedrock::version::v1_26_51::McpePacketData;

    use super::*;

    #[test]
    fn client_close_uses_native_unspecified_type() {
        let packet = container_close_packet(4).unwrap();
        let McpePacketData::ContainerClosePacket(close) = packet.data else {
            panic!("expected ContainerClose packet");
        };
        assert_eq!(close.container_id, 4);
        assert_eq!(close.container_type, 0xf7);
        assert!(!close.server_initiated_close);
    }

    #[test]
    fn personal_inventory_open_targets_self_without_a_position() {
        let packet = open_inventory_packet(42).unwrap();
        let McpePacketData::InteractPacket(interact) = packet.data else {
            panic!("expected Interact packet");
        };
        assert_eq!(interact.action, InteractAction::Openinventory);
        assert_eq!(interact.target_runtime_id.actor_runtime_id, 42);
        assert_eq!(interact.position, None);
        assert_eq!(
            open_inventory_packet(0).unwrap_err(),
            InventoryPacketError::InvalidInventoryTargetRuntimeId
        );
    }
}
