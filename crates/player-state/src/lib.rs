//! Inventory and local-player facts shared by UI and gameplay.

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
}
