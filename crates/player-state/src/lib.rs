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

    /// Captures the ledger as painting must show it, before this frame's sends.
    #[must_use]
    pub fn capture_ledger(&self) -> CapturedLedger {
        CapturedLedger(self.inventory.ledger().clone())
    }

    /// Returns a read-only copy of this state showing `captured` in place of the live ledger.
    #[must_use]
    pub fn present(&self, captured: CapturedLedger) -> PresentedPlayer {
        PresentedPlayer(Self {
            inventory: self.inventory.with_ledger(captured.0),
            facts: self.facts.clone(),
        })
    }
}

/// A ledger copy taken at a frame boundary; opaque until handed back to [`PlayerState::present`].
pub struct CapturedLedger(inventory::PlayerInventoryLedger);

/// Player state for painting; it can be read but never feeds back into authority.
pub struct PresentedPlayer(PlayerState);

impl std::ops::Deref for PresentedPlayer {
    type Target = PlayerState;

    fn deref(&self) -> &PlayerState {
        &self.0
    }
}

#[cfg(test)]
mod tests {
    use super::PlayerState;

    // Painting sees the captured ledger beside live session state; authority keeps its ledger.
    #[test]
    fn presented_player_shows_captured_ledger_without_touching_authority() {
        let mut live = PlayerState::new(1);
        let captured = live.capture_ledger();
        let before_send = format!("{:?}", live.inventory.ledger());
        live.inventory.ledger_mut().begin_session(2);
        live.inventory.set_local_selected_slot(3);
        let after_send = format!("{:?}", live.inventory.ledger());
        assert_ne!(before_send, after_send);

        let presented = live.present(captured);
        assert_eq!(format!("{:?}", presented.inventory.ledger()), before_send);
        assert_eq!(presented.inventory.selected_hotbar_slot(None), Some(3));
        assert_eq!(format!("{:?}", live.inventory.ledger()), after_send);
    }
}
