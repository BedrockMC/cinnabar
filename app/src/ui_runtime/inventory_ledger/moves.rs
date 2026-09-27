//! Cursor-free moves: number-key hotbar swaps, drops and quick moves.
//!
//! Hotbar swaps follow the owner's vanilla-mirroring rule (Swap between two
//! occupied cells, otherwise one Place) and drops use one Drop action. A quick
//! move sends the single Place its capture shows; which destination it picks
//! is provisional pending vanilla evidence.

use protocol::StackRequestAction;

use super::cells::{Cell, Held};
use super::gesture::{
    Built, InventoryTarget, StackRequestActionKind, Submission, counted_merge, counted_transfer,
    has_meaningful_overlay, swap,
};
use super::helpers::request_slot;
use super::overlay::DeltaGroup;
use super::registry::OccupiedStackRelation;
use super::{InventoryGestureError, PlayerInventoryLedger};

/// What a drop takes from.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum DropSource {
    Target(InventoryTarget),
    Cursor,
}

impl PlayerInventoryLedger {
    /// Swaps a hovered cell with hotbar cell `hotbar`, or places into
    /// whichever of the two is empty.
    pub fn begin_hotbar_swap(
        &mut self,
        target: InventoryTarget,
        hotbar: u8,
    ) -> Result<i32, InventoryGestureError> {
        let (source, destination) = (target.cell(), Cell::Inventory(hotbar));
        if hotbar >= 9 || source == destination {
            return Err(InventoryGestureError::InvalidRequest);
        }
        let personal_generation = self.gesture_preflight(!matches!(source, Cell::Storage(_)))?;
        self.check_surfaces([source, destination])?;
        let from = self.movable(source)?;
        let to = self.movable(destination)?;
        let identity = self.storage_identity();
        let built = match (from, to) {
            (Some(from), Some(to)) => swap(source, destination, &from.stack, &to.stack, identity)?,
            (Some(from), None) => place(source, destination, &from, identity)?,
            (None, Some(to)) => place(destination, source, &to, identity)?,
            (None, None) => return Err(InventoryGestureError::EmptyGesture),
        };
        self.submit_built(built, personal_generation)
    }

    /// Drops `amount` (or the whole stack) from a cell or the cursor.
    pub fn begin_drop(
        &mut self,
        source: DropSource,
        amount: Option<u16>,
    ) -> Result<i32, InventoryGestureError> {
        let cell = match source {
            DropSource::Target(target) => target.cell(),
            DropSource::Cursor => Cell::Cursor,
        };
        let personal_generation = self.gesture_preflight(!matches!(cell, Cell::Storage(_)))?;
        self.check_surfaces([cell])?;
        let held = match cell {
            Cell::Cursor => self.named(self.view().get(Cell::Cursor).cloned())?,
            cell => self.movable(cell)?,
        }
        .ok_or(InventoryGestureError::EmptyGesture)?;
        let amount = amount.unwrap_or(held.stack.count);
        let wire = u8::try_from(amount)
            .ok()
            .filter(|amount| *amount != 0 && u16::from(*amount) <= held.stack.count)
            .ok_or(InventoryGestureError::InvalidRequest)?;
        let id = held.stack.stack_network_id;
        let built = Built {
            action: StackRequestAction::Drop {
                amount: wire,
                source: request_slot(cell, id, self.storage_identity())?,
                randomly: false,
            },
            group: DeltaGroup::Shrink {
                source: cell,
                amount,
                source_id: id,
            },
            requires_distinct_stack_ids: false,
            registry_bound_merge: false,
        };
        self.submit_built(built, personal_generation)
    }

    /// Moves a hovered stack into the first cell of the opposite range that
    /// can take it: a compatible partial stack first, else an empty cell.
    pub fn begin_quick_move(
        &mut self,
        target: InventoryTarget,
    ) -> Result<i32, InventoryGestureError> {
        let source = target.cell();
        let personal_generation = self.gesture_preflight(!matches!(source, Cell::Storage(_)))?;
        self.check_surfaces([source])?;
        let from = self
            .movable(source)?
            .ok_or(InventoryGestureError::EmptyGesture)?;
        let identity = self.storage_identity();
        let range = self.quick_move_range(source);
        let merge = range.iter().find_map(|cell| {
            let into = self.view().get(*cell)?;
            if self.awaiting_identity(into)
                || has_meaningful_overlay(from.overlay.as_ref())
                || has_meaningful_overlay(into.overlay.as_ref())
            {
                return None;
            }
            match self.occupied_stack_relation(&from.stack, &into.stack) {
                OccupiedStackRelation::Compatible { capacity } if into.stack.count < capacity => {
                    Some((*cell, into.stack.clone(), capacity))
                }
                _ => None,
            }
        });
        let built = if let Some((cell, into, capacity)) = merge {
            let amount = from.stack.count.min(capacity - into.count);
            counted_merge(
                StackRequestActionKind::Place,
                source,
                cell,
                &from.stack,
                &into,
                identity,
                amount,
                capacity,
            )?
        } else {
            let empty = range
                .into_iter()
                .find(|cell| self.view().get(*cell).is_none())
                .ok_or(InventoryGestureError::InvalidRequest)?;
            place(source, empty, &from, identity)?
        };
        self.submit_built(built, personal_generation)
    }

    /// Candidate destinations, in order, for a quick move out of `source`.
    fn quick_move_range(&self, source: Cell) -> Vec<Cell> {
        let player = |range: std::ops::Range<u8>| {
            range
                .filter(|slot| self.known[usize::from(*slot)])
                .map(Cell::Inventory)
                .collect::<Vec<_>>()
        };
        let storage_open = self
            .storage
            .as_ref()
            .is_some_and(|storage| storage.identity.is_some());
        match source {
            Cell::Inventory(_) if storage_open => (0..u8::MAX)
                .map(Cell::Storage)
                .take_while(|cell| self.confirmed.contains(*cell))
                .collect(),
            Cell::Inventory(slot) if slot < 9 => player(9..36),
            Cell::Inventory(_) => player(0..9),
            _ => {
                let mut cells = player(9..36);
                cells.extend(player(0..9));
                cells
            }
        }
    }

    /// The current stack in a validated gesture cell, refusing one that
    /// cannot be named in a request yet.
    fn movable(&self, cell: Cell) -> Result<Option<Held>, InventoryGestureError> {
        self.check_target(cell)?;
        self.named(self.view().get(cell).cloned())
    }

    fn named(&self, held: Option<Held>) -> Result<Option<Held>, InventoryGestureError> {
        match held {
            Some(held) if self.awaiting_identity(&held) => {
                Err(InventoryGestureError::AwaitingIdentity)
            }
            held => Ok(held),
        }
    }

    fn submit_built(
        &mut self,
        built: Built,
        personal_generation: Option<u64>,
    ) -> Result<i32, InventoryGestureError> {
        self.submit(Submission {
            actions: vec![built.action],
            groups: vec![built.group],
            personal_generation,
            requires_distinct_stack_ids: built.requires_distinct_stack_ids,
            registry_bound_merge: built.registry_bound_merge,
        })
    }
}

/// One Place of a whole stack into an empty cell.
fn place(
    source: Cell,
    destination: Cell,
    held: &Held,
    identity: Option<protocol::ContainerIdentity>,
) -> Result<Built, InventoryGestureError> {
    counted_transfer(
        StackRequestActionKind::Place,
        source,
        destination,
        &held.stack,
        identity,
        None,
    )
}
