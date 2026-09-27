//! Retained cell values shared by confirmed server truth and the folded view.

use protocol::NetworkItemStack;

use super::{PLAYER_INVENTORY_SLOT_COUNT, StackResponseOverlay};

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub(super) enum Cell {
    Inventory(u8),
    Storage(u8),
    Cursor,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub(super) enum CellSurface {
    Player,
    Storage,
    Cursor,
}

impl Cell {
    pub(super) const fn surface(self) -> CellSurface {
        match self {
            Self::Inventory(_) => CellSurface::Player,
            Self::Storage(_) => CellSurface::Storage,
            Self::Cursor => CellSurface::Cursor,
        }
    }
}

/// One non-empty stack plus the response overlay that travels with it.
#[derive(Debug, Clone, Eq, PartialEq)]
pub(super) struct Held {
    pub(super) stack: NetworkItemStack,
    pub(super) overlay: Option<StackResponseOverlay>,
}

impl Held {
    /// `None` for an empty wire stack; empty cells are never retained as values.
    pub(super) fn new(stack: &NetworkItemStack) -> Option<Self> {
        (!stack.is_empty()).then(|| Self {
            stack: stack.clone(),
            overlay: None,
        })
    }
}

#[derive(Debug, Clone)]
pub(super) struct Cells {
    player: [Option<Held>; PLAYER_INVENTORY_SLOT_COUNT],
    cursor: Option<Held>,
    storage: Vec<Option<Held>>,
}

impl Default for Cells {
    fn default() -> Self {
        Self {
            player: std::array::from_fn(|_| None),
            cursor: None,
            storage: Vec::new(),
        }
    }
}

impl Cells {
    pub(super) fn get(&self, cell: Cell) -> Option<&Held> {
        match cell {
            Cell::Inventory(slot) => self.player.get(usize::from(slot))?.as_ref(),
            Cell::Storage(slot) => self.storage.get(usize::from(slot))?.as_ref(),
            Cell::Cursor => self.cursor.as_ref(),
        }
    }

    pub(super) fn get_mut(&mut self, cell: Cell) -> Option<&mut Held> {
        self.entry(cell)?.as_mut()
    }

    /// Whether `cell` addresses a retained position at all.
    pub(super) fn contains(&self, cell: Cell) -> bool {
        match cell {
            Cell::Inventory(slot) => usize::from(slot) < PLAYER_INVENTORY_SLOT_COUNT,
            Cell::Storage(slot) => usize::from(slot) < self.storage.len(),
            Cell::Cursor => true,
        }
    }

    /// Writes one cell; out-of-range addresses are ignored and report `false`.
    pub(super) fn set(&mut self, cell: Cell, value: Option<Held>) -> bool {
        match self.entry(cell) {
            Some(entry) => {
                *entry = value;
                true
            }
            None => false,
        }
    }

    pub(super) fn take(&mut self, cell: Cell) -> Option<Held> {
        self.entry(cell)?.take()
    }

    fn entry(&mut self, cell: Cell) -> Option<&mut Option<Held>> {
        match cell {
            Cell::Inventory(slot) => self.player.get_mut(usize::from(slot)),
            Cell::Storage(slot) => self.storage.get_mut(usize::from(slot)),
            Cell::Cursor => Some(&mut self.cursor),
        }
    }

    pub(super) fn replace_storage(&mut self, slots: &[NetworkItemStack]) {
        self.storage = slots.iter().map(Held::new).collect();
    }

    pub(super) fn storage_len(&self) -> usize {
        self.storage.len()
    }

    pub(super) fn clear_storage(&mut self) {
        self.storage = Vec::new();
    }

    /// Every occupied retained cell in address order.
    pub(super) fn occupied(&self) -> impl Iterator<Item = (Cell, &Held)> {
        let player = self
            .player
            .iter()
            .enumerate()
            .filter_map(|(slot, held)| Some((Cell::Inventory(slot as u8), held.as_ref()?)));
        let cursor = self.cursor.as_ref().map(|held| (Cell::Cursor, held));
        let storage = self
            .storage
            .iter()
            .enumerate()
            .filter_map(|(slot, held)| Some((Cell::Storage(slot as u8), held.as_ref()?)));
        player.chain(cursor).chain(storage)
    }
}
