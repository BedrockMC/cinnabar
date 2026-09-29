use protocol::{StackRequestContainer, StackRequestSlot};

use super::{Cell, ContainerIdentity, InventoryGestureError, StorageWindow};

pub(super) const fn valid_raw_window_id(window_id: i32) -> bool {
    matches!(window_id, -128..=255)
}

pub(super) const fn valid_storage_window_id(window_id: i32) -> bool {
    window_id != 0 && valid_raw_window_id(window_id)
}

pub(super) fn storage_slot_identity_matches(
    storage: &StorageWindow,
    identity: ContainerIdentity,
) -> bool {
    if identity.window_id != Some(storage.window_id) {
        return false;
    }
    match (identity.slot_type, identity.dynamic_id) {
        (None, None) => true,
        _ => storage.identity == Some(identity),
    }
}

/// Whether one container identity the projection left unrouted is exactly the
/// prior bare-window storage leg: no decoded container name, no dynamic id,
/// and a window id matching the one open generic-storage window. Named or
/// dynamic identities never qualify — they must project canonically first.
pub(super) fn bare_storage_window_matches(
    storage: Option<&StorageWindow>,
    identity: &ContainerIdentity,
) -> bool {
    identity.slot_type.is_none()
        && identity.dynamic_id.is_none()
        && storage.is_some_and(|storage| storage_slot_identity_matches(storage, *identity))
}

/// How the open window's own cells are named in a request.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub(super) struct WindowAddress {
    pub(super) identity: ContainerIdentity,
    pub(super) kind: protocol::WindowKind,
}

pub(super) fn request_slot(
    cell: Cell,
    stack_network_id: i32,
    window: Option<WindowAddress>,
) -> Result<StackRequestSlot, InventoryGestureError> {
    let (container, slot) = match cell {
        Cell::Inventory(slot) => (StackRequestContainer::PlayerInventory, slot),
        Cell::Cursor => (StackRequestContainer::Cursor, 0),
        Cell::Storage(slot) => {
            let window = window.ok_or(InventoryGestureError::InvalidRequest)?;
            protocol::open_cell_request(
                window.kind,
                slot,
                window.identity.dynamic_id,
                window.identity.slot_type,
            )
            .ok_or(InventoryGestureError::InvalidRequest)?
        }
        Cell::Armor(slot) => (StackRequestContainer::Armor, slot),
        Cell::Offhand => (StackRequestContainer::Offhand, 1),
        Cell::Craft(slot) => (
            protocol::ui_slot_request_container(slot)
                .ok_or(InventoryGestureError::InvalidRequest)?,
            slot,
        ),
        Cell::CreatedOutput => (
            StackRequestContainer::CreatedOutput,
            protocol::CREATED_OUTPUT_SLOT,
        ),
    };
    Ok(StackRequestSlot {
        container,
        slot,
        stack_network_id,
    })
}
