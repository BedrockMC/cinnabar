//! Inventory and local-player facts shared by UI and gameplay.

use client_world::ingestion::NetworkItemStack;
use std::sync::Arc;

/// Domain authority shared synchronously by network input, UI commands and movement.
#[derive(Clone, Debug)]
pub struct PlayerState {
    pub inventory: inventory::InventorySession,
    pub facts: client_world::LocalPlayerFacts,
}

impl PlayerState {
    /// Starts both domain owners at the same session generation.
    pub fn new(session: u64) -> Self {
        Self {
            inventory: inventory::InventorySession::new(session),
            facts: client_world::LocalPlayerFacts::new(session),
        }
    }

    /// Retires both owners together when the network session changes.
    pub fn begin_session(&mut self, session: u64) {
        self.inventory.begin_session(session);
        self.facts.begin_session(session);
    }

    /// The selected hotbar slot under the current game mode.
    pub fn selected_hotbar_slot(&self) -> Option<u8> {
        self.inventory
            .selected_hotbar_slot(self.facts.player_game_mode())
    }

    /// The selected slot and its tri-state stack authority.
    pub fn selected_stack_snapshot(&self) -> Option<inventory::SelectedStackSnapshot<'_>> {
        self.inventory
            .selected_stack_snapshot(self.facts.player_game_mode())
    }

    /// The selected stack, when one is present.
    pub fn selected_stack(&self) -> Option<&NetworkItemStack> {
        self.inventory.selected_stack(self.facts.player_game_mode())
    }

    /// The custom name the selected hotbar cell presents, following the predicted stack.
    pub fn selected_stack_custom_name(&self) -> Option<Arc<str>> {
        self.inventory
            .selected_stack_custom_name(self.facts.player_game_mode())
    }

    /// The stack presented in one hotbar cell.
    pub fn presented_hotbar_stack(&self, slot: u8) -> Option<&NetworkItemStack> {
        self.inventory
            .presented_hotbar_stack(slot, self.facts.player_game_mode())
    }
}
