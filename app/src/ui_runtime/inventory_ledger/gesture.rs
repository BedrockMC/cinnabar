use protocol::{ContainerIdentity, NetworkItemStack, StackRequestAction, StackRequestSlot};

use super::helpers::request_slot;
use super::overlay::DeltaGroup;
use super::registry::OccupiedStackRelation;
use super::{
    Cell, InventoryGestureError, InventoryPendingState, PLAYER_INVENTORY_SLOT_COUNT,
    PendingRequest, PlayerInventoryLedger, StackResponseOverlay,
};

/// One pointer gesture against a single cell and the cursor.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum CellGesture {
    /// Take, place, merge or swap the whole stack.
    Click,
    TakeCount(u16),
    PlaceCount(u16),
}

/// A cell a player gesture may target.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum InventoryTarget {
    Player(u8),
    Storage(u8),
    Armor(u8),
    Offhand,
    /// A crafting cell by UI inventory slot, 28..=40.
    Craft(u8),
}

impl InventoryTarget {
    pub(super) const fn cell(self) -> Cell {
        match self {
            Self::Player(slot) => Cell::Inventory(slot),
            Self::Storage(slot) => Cell::Storage(slot),
            Self::Armor(slot) => Cell::Armor(slot),
            Self::Offhand => Cell::Offhand,
            Self::Craft(slot) => Cell::Craft(slot),
        }
    }
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
enum StackRequestActionKind {
    Take,
    Place,
}

/// A built request before it receives an id.
struct Built {
    action: StackRequestAction,
    group: DeltaGroup,
    requires_distinct_stack_ids: bool,
    registry_bound_merge: bool,
}

fn transfer_action(
    kind: StackRequestActionKind,
    amount: u16,
    source: StackRequestSlot,
    destination: StackRequestSlot,
) -> Result<StackRequestAction, InventoryGestureError> {
    let amount = u8::try_from(amount)
        .ok()
        .filter(|amount| *amount != 0)
        .ok_or(InventoryGestureError::InvalidRequest)?;
    Ok(match kind {
        StackRequestActionKind::Take => StackRequestAction::Take {
            amount,
            source,
            destination,
        },
        StackRequestActionKind::Place => StackRequestAction::Place {
            amount,
            source,
            destination,
        },
    })
}

/// Moves `requested` (or the whole stack) into an empty destination.
fn counted_transfer(
    kind: StackRequestActionKind,
    source: Cell,
    destination: Cell,
    stack: &NetworkItemStack,
    storage_identity: Option<ContainerIdentity>,
    requested: Option<u16>,
) -> Result<Built, InventoryGestureError> {
    let amount = requested.unwrap_or(stack.count);
    if amount == 0 || amount > stack.count {
        return Err(InventoryGestureError::InvalidRequest);
    }
    let action = transfer_action(
        kind,
        amount,
        request_slot(source, stack.stack_network_id, storage_identity)?,
        request_slot(destination, 0, storage_identity)?,
    )?;
    Ok(Built {
        action,
        group: DeltaGroup::Transfer {
            source,
            destination,
            amount,
            source_id: stack.stack_network_id,
            destination_id: None,
            capacity: None,
        },
        requires_distinct_stack_ids: amount < stack.count,
        registry_bound_merge: false,
    })
}

/// Merges `amount` into an occupied compatible destination.
#[allow(clippy::too_many_arguments)]
fn counted_merge(
    kind: StackRequestActionKind,
    source: Cell,
    destination: Cell,
    source_stack: &NetworkItemStack,
    destination_stack: &NetworkItemStack,
    storage_identity: Option<ContainerIdentity>,
    amount: u16,
    capacity: u16,
) -> Result<Built, InventoryGestureError> {
    if amount == 0
        || amount > source_stack.count
        || destination_stack
            .count
            .checked_add(amount)
            .is_none_or(|count| count > capacity)
    {
        return Err(InventoryGestureError::InvalidRequest);
    }
    let action = transfer_action(
        kind,
        amount,
        request_slot(source, source_stack.stack_network_id, storage_identity)?,
        request_slot(
            destination,
            destination_stack.stack_network_id,
            storage_identity,
        )?,
    )?;
    Ok(Built {
        action,
        group: DeltaGroup::Transfer {
            source,
            destination,
            amount,
            source_id: source_stack.stack_network_id,
            destination_id: Some(destination_stack.stack_network_id),
            capacity: Some(capacity),
        },
        requires_distinct_stack_ids: amount < source_stack.count,
        registry_bound_merge: true,
    })
}

fn swap(
    source: Cell,
    destination: Cell,
    source_stack: &NetworkItemStack,
    destination_stack: &NetworkItemStack,
    storage_identity: Option<ContainerIdentity>,
) -> Result<Built, InventoryGestureError> {
    Ok(Built {
        action: StackRequestAction::Swap {
            source: request_slot(source, source_stack.stack_network_id, storage_identity)?,
            destination: request_slot(
                destination,
                destination_stack.stack_network_id,
                storage_identity,
            )?,
        },
        group: DeltaGroup::Swap {
            source,
            destination,
            source_id: source_stack.stack_network_id,
            destination_id: destination_stack.stack_network_id,
        },
        requires_distinct_stack_ids: false,
        registry_bound_merge: false,
    })
}

fn has_meaningful_overlay(overlay: Option<&StackResponseOverlay>) -> bool {
    overlay.is_some_and(|overlay| {
        overlay.custom_name.is_some()
            || overlay.filtered_custom_name.is_some()
            || overlay.durability_correction.is_some()
    })
}

impl PlayerInventoryLedger {
    /// Queues one gesture on any retained target cell.
    pub fn begin_target_gesture(
        &mut self,
        target: InventoryTarget,
        gesture: CellGesture,
    ) -> Result<i32, InventoryGestureError> {
        self.begin_cell_gesture(target.cell(), gesture)
    }

    pub fn begin_click(&mut self, slot: u8) -> Result<i32, InventoryGestureError> {
        self.begin_cell_gesture(Cell::Inventory(slot), CellGesture::Click)
    }

    pub fn begin_storage_click(&mut self, slot: u8) -> Result<i32, InventoryGestureError> {
        self.begin_cell_gesture(Cell::Storage(slot), CellGesture::Click)
    }

    pub fn begin_take_count(
        &mut self,
        slot: u8,
        amount: u16,
    ) -> Result<i32, InventoryGestureError> {
        self.begin_cell_gesture(Cell::Inventory(slot), CellGesture::TakeCount(amount))
    }

    pub fn begin_place_count(
        &mut self,
        slot: u8,
        amount: u16,
    ) -> Result<i32, InventoryGestureError> {
        self.begin_cell_gesture(Cell::Inventory(slot), CellGesture::PlaceCount(amount))
    }

    pub fn begin_storage_take_count(
        &mut self,
        slot: u8,
        amount: u16,
    ) -> Result<i32, InventoryGestureError> {
        self.begin_cell_gesture(Cell::Storage(slot), CellGesture::TakeCount(amount))
    }

    pub fn begin_storage_place_count(
        &mut self,
        slot: u8,
        amount: u16,
    ) -> Result<i32, InventoryGestureError> {
        self.begin_cell_gesture(Cell::Storage(slot), CellGesture::PlaceCount(amount))
    }

    fn begin_cell_gesture(
        &mut self,
        target: Cell,
        gesture: CellGesture,
    ) -> Result<i32, InventoryGestureError> {
        if self.authority != Some(protocol::InventoryAuthority::Server) {
            return Err(InventoryGestureError::AuthorityUnavailable);
        }
        // A locally closing window only waits out its admitted requests.
        if self.resync_required() || self.storage.as_ref().is_some_and(|storage| storage.closing) {
            return Err(InventoryGestureError::ResyncRequired);
        }
        self.ensure_queue_capacity()?;
        let personal_generation = if matches!(target, Cell::Inventory(_)) && self.storage.is_none()
        {
            Some(
                self.personal_generation_for_gesture()
                    .ok_or(InventoryGestureError::PersonalInventoryUnavailable)?,
            )
        } else {
            None
        };
        match target {
            Cell::Inventory(slot) => {
                let index = usize::from(slot);
                if index >= PLAYER_INVENTORY_SLOT_COUNT {
                    return Err(InventoryGestureError::InvalidSlot(slot));
                }
                if !self.known[index] {
                    return Err(InventoryGestureError::UnknownSlot(slot));
                }
            }
            Cell::Storage(slot) => {
                let storage = self
                    .storage
                    .as_ref()
                    .ok_or(InventoryGestureError::InvalidStorageSlot(slot))?;
                if storage.identity.is_none() || storage.resync_required {
                    return Err(InventoryGestureError::ResyncRequired);
                }
                if !self.confirmed.contains(target) {
                    return Err(InventoryGestureError::InvalidStorageSlot(slot));
                }
            }
            Cell::Armor(_) | Cell::Offhand | Cell::Craft(_) => {
                if !self.confirmed.contains(target) {
                    return Err(InventoryGestureError::InvalidRequest);
                }
            }
            Cell::Cursor | Cell::CreatedOutput => {
                return Err(InventoryGestureError::InvalidRequest);
            }
        }
        let target_held = self.view().get(target).cloned();
        let cursor_held = self.view().get(Cell::Cursor).cloned();
        if [target_held.as_ref(), cursor_held.as_ref()]
            .into_iter()
            .flatten()
            .any(|held| self.awaiting_identity(held))
        {
            return Err(InventoryGestureError::AwaitingIdentity);
        }
        let storage_identity = self.storage_identity();
        let target_overlay = target_held.as_ref().and_then(|held| held.overlay.as_ref());
        let cursor_overlay = cursor_held.as_ref().and_then(|held| held.overlay.as_ref());
        let target_stack = target_held.as_ref().map(|held| &held.stack);
        let cursor_stack = cursor_held.as_ref().map(|held| &held.stack);
        let plain_overlays =
            !has_meaningful_overlay(target_overlay) && !has_meaningful_overlay(cursor_overlay);
        let built = match gesture {
            CellGesture::Click => match (target_stack, cursor_stack) {
                (Some(stack), None) => counted_transfer(
                    StackRequestActionKind::Take,
                    target,
                    Cell::Cursor,
                    stack,
                    storage_identity,
                    None,
                )?,
                (None, Some(stack)) => counted_transfer(
                    StackRequestActionKind::Place,
                    Cell::Cursor,
                    target,
                    stack,
                    storage_identity,
                    None,
                )?,
                (Some(inventory), Some(cursor)) => {
                    match self.occupied_stack_relation(cursor, inventory) {
                        OccupiedStackRelation::Compatible { capacity } if plain_overlays => {
                            let amount = cursor.count.min(capacity.saturating_sub(inventory.count));
                            counted_merge(
                                StackRequestActionKind::Place,
                                Cell::Cursor,
                                target,
                                cursor,
                                inventory,
                                storage_identity,
                                amount,
                                capacity,
                            )?
                        }
                        OccupiedStackRelation::Incompatible => {
                            swap(Cell::Cursor, target, cursor, inventory, storage_identity)?
                        }
                        OccupiedStackRelation::Compatible { .. }
                        | OccupiedStackRelation::Unsupported => {
                            return Err(InventoryGestureError::InvalidRequest);
                        }
                    }
                }
                (None, None) => return Err(InventoryGestureError::EmptyGesture),
            },
            CellGesture::TakeCount(amount) => {
                let stack = target_stack.ok_or(InventoryGestureError::EmptyGesture)?;
                match cursor_stack {
                    Some(cursor) => {
                        let OccupiedStackRelation::Compatible { capacity } =
                            self.occupied_stack_relation(stack, cursor)
                        else {
                            return Err(InventoryGestureError::InvalidRequest);
                        };
                        if !plain_overlays {
                            return Err(InventoryGestureError::InvalidRequest);
                        }
                        counted_merge(
                            StackRequestActionKind::Take,
                            target,
                            Cell::Cursor,
                            stack,
                            cursor,
                            storage_identity,
                            amount,
                            capacity,
                        )?
                    }
                    None => counted_transfer(
                        StackRequestActionKind::Take,
                        target,
                        Cell::Cursor,
                        stack,
                        storage_identity,
                        Some(amount),
                    )?,
                }
            }
            CellGesture::PlaceCount(amount) => {
                let stack = cursor_stack.ok_or(InventoryGestureError::EmptyGesture)?;
                match target_stack {
                    Some(inventory) => {
                        let OccupiedStackRelation::Compatible { capacity } =
                            self.occupied_stack_relation(stack, inventory)
                        else {
                            return Err(InventoryGestureError::InvalidRequest);
                        };
                        if !plain_overlays {
                            return Err(InventoryGestureError::InvalidRequest);
                        }
                        counted_merge(
                            StackRequestActionKind::Place,
                            Cell::Cursor,
                            target,
                            stack,
                            inventory,
                            storage_identity,
                            amount,
                            capacity,
                        )?
                    }
                    None => counted_transfer(
                        StackRequestActionKind::Place,
                        Cell::Cursor,
                        target,
                        stack,
                        storage_identity,
                        Some(amount),
                    )?,
                }
            }
        };
        let request_id = self.next_request_id;
        self.next_request_id = self
            .next_request_id
            .checked_sub(2)
            .ok_or(InventoryGestureError::InvalidRequest)?;
        self.enqueue(PendingRequest {
            request_id,
            actions: vec![built.action],
            groups: vec![built.group],
            state: InventoryPendingState::AwaitingTransport,
            transport_deadline_millis: None,
            deadline_millis: None,
            timed_out: false,
            accepted: None,
            session_generation: self.session_generation,
            storage_generation: self.storage.as_ref().map(|storage| storage.generation),
            personal_generation,
            storage_identity,
            requires_distinct_stack_ids: built.requires_distinct_stack_ids,
            registry_bound_merge: built.registry_bound_merge,
            predicted: Vec::new(),
        });
        Ok(request_id)
    }
}
