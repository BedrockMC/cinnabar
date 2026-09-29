use protocol::{InventoryEvent, ItemRegistryEvent};

use super::{MAX_PENDING_INVENTORY_EVENTS, UiRuntime, UiRuntimeError};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SequencedInventoryEvent {
    pub session_generation: u64,
    pub fifo_sequence: u64,
    pub event: InventoryAuthorityEvent,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InventoryAuthorityEvent {
    Inventory(InventoryEvent),
    Registry(ItemRegistryEvent),
}

impl UiRuntime {
    pub(crate) fn enqueue_inventory_event(
        &mut self,
        session_generation: u64,
        fifo_sequence: u64,
        event: InventoryEvent,
    ) -> Result<(), UiRuntimeError> {
        self.enqueue_inventory_authority_event(
            session_generation,
            fifo_sequence,
            InventoryAuthorityEvent::Inventory(event),
        )
    }

    pub(crate) fn enqueue_item_registry_event(
        &mut self,
        session_generation: u64,
        fifo_sequence: u64,
        event: ItemRegistryEvent,
    ) -> Result<(), UiRuntimeError> {
        self.enqueue_inventory_authority_event(
            session_generation,
            fifo_sequence,
            InventoryAuthorityEvent::Registry(event),
        )
    }

    fn enqueue_inventory_authority_event(
        &mut self,
        session_generation: u64,
        fifo_sequence: u64,
        event: InventoryAuthorityEvent,
    ) -> Result<(), UiRuntimeError> {
        if session_generation != self.session_id {
            return Err(UiRuntimeError::WrongSession {
                expected: self.session_id,
                actual: session_generation,
            });
        }
        if let Some(previous) = self.last_inventory_sequence
            && fifo_sequence <= previous
        {
            return Err(UiRuntimeError::StaleFifoSequence {
                previous,
                actual: fifo_sequence,
            });
        }
        if self.pending_inventory.len() >= MAX_PENDING_INVENTORY_EVENTS {
            return Err(UiRuntimeError::InventoryQueueFull {
                maximum: MAX_PENDING_INVENTORY_EVENTS,
            });
        }
        self.crafting_authority.note_ingress(fifo_sequence, &event);
        self.pending_inventory.push_back(SequencedInventoryEvent {
            session_generation,
            fifo_sequence,
            event,
        });
        self.last_inventory_sequence = Some(fifo_sequence);
        Ok(())
    }

    pub fn pop_inventory_event(&mut self) -> Option<SequencedInventoryEvent> {
        let event = self.pending_inventory.pop_front()?;
        self.crafting_authority.bypass(
            self.last_inventory_sequence
                .unwrap_or(event.fifo_sequence)
                .max(event.fifo_sequence),
        );
        Some(event)
    }

    pub(crate) fn synchronize_crafting_frontier(
        &mut self,
        session: u64,
        identity: Option<(u64, u64, Option<u64>)>,
    ) {
        self.crafting_authority
            .synchronize(if session == self.session_id {
                identity
            } else {
                None
            });
    }

    pub(crate) fn publish_crafting_bootstrap(
        &mut self,
        registry: Option<&ItemRegistryEvent>,
        authority: protocol::InventoryAuthority,
    ) {
        self.crafting_authority.bootstrap(registry, authority);
    }

    /// Borrowed display values only, with immutable credit owners retained by this runtime.
    /// Inactive crafting authority never allocates a request or sends a packet.
    pub fn crafting_preview(&self) -> Option<super::CraftingPreview<'_>> {
        self.crafting_authority.preview()
    }

    /// The recipe the presented crafting grid forms against the committed
    /// catalog; `Unavailable` without a catalog or known grid identities.
    #[must_use]
    pub fn crafting_match(&self) -> protocol::CraftGridMatch {
        let ledger = self.inventory_ledger();
        let (Some(catalog), Some(cells)) = (
            self.crafting_authority.catalog(),
            ledger.crafting_grid_cells(),
        ) else {
            return protocol::CraftGridMatch::Unavailable;
        };
        if cells.iter().all(Option::is_none) {
            return protocol::CraftGridMatch::NoMatch;
        }
        let items: Vec<_> = cells
            .iter()
            .map(|cell| {
                cell.as_ref()
                    .map(super::inventory_ledger::CraftGridCell::item)
            })
            .collect();
        protocol::match_crafting_grid(catalog, ledger.crafting_grid().width(), &items)
    }

    /// Crafts the grid's unique recipe once into the cursor.
    pub fn begin_crafting(
        &mut self,
    ) -> Result<i32, super::inventory_ledger::InventoryGestureError> {
        self.begin_crafting_into(super::inventory_ledger::CraftSink::Cursor)
    }

    /// Crafts the grid's unique recipe once into `sink`.
    pub fn begin_crafting_into(
        &mut self,
        sink: super::inventory_ledger::CraftSink,
    ) -> Result<i32, super::inventory_ledger::InventoryGestureError> {
        let protocol::CraftGridMatch::Unique(recipe) = self.crafting_match() else {
            return Err(super::inventory_ledger::InventoryGestureError::InvalidRequest);
        };
        self.inventory_ledger_mut().begin_craft_into(&recipe, 1, sink)
    }

    /// Crafts the grid's unique recipe as many times as it fits, shift-click style.
    pub fn begin_crafting_all(
        &mut self,
    ) -> Result<i32, super::inventory_ledger::InventoryGestureError> {
        let protocol::CraftGridMatch::Unique(recipe) = self.crafting_match() else {
            return Err(super::inventory_ledger::InventoryGestureError::InvalidRequest);
        };
        self.inventory_ledger_mut().begin_craft_all(&recipe)
    }
}
