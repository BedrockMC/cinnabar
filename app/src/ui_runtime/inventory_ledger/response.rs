//! Authoritative item-stack responses plus the retained per-cell response
//! overlay.
//!
//! An accepted correction is authoritative for its whole cell: beyond the
//! count/stack-network-id corrections applied directly to the retained
//! stack, the server's custom display names and durability damage are kept
//! as one [`StackResponseOverlay`] keyed to that cell. The overlay travels
//! with its predicted stack through a pending gesture. Each correction
//! restates only what changed: empty name halves and nonpositive durability
//! are the wire's unstated encodings, so they never fabricate facts — a
//! field stays absent until some accepted correction states it, a stated
//! value replaces an earlier one, an omitted field keeps what the cell
//! already retained, and presentation falls back to local derivation for
//! every absent field. That guarantee is scoped deliberately: a well-formed
//! accepted correction that restates a changed positive stack-network id
//! updates the retained stack in place, and this module does not claim that
//! unstated overlay fields follow such an id change. Rejected requests roll
//! back without writing one.

use std::sync::Arc;

use protocol::{ItemStackResponseEvent, StackResponseSlot, StackResponseStatus};

use super::{Cell, PlayerInventoryLedger};

/// Authoritative presentation facts an accepted server correction attached
/// to one inventory cell: custom display names plus the exact durability
/// damage. Every field is `None` while unstated, so absent facts stay
/// genuinely absent and presentation keeps its local derivation instead of
/// reading a defaulted value as authoritative. The overlay never alters
/// stack identity; replacing the cell through any other authoritative path
/// drops it.
#[derive(Debug, Clone, Default, Eq, PartialEq)]
pub struct StackResponseOverlay {
    /// Server-owned display name once a response states it.
    pub custom_name: Option<Arc<str>>,
    /// Redacted half of the same redactable wire string pair once stated.
    pub filtered_custom_name: Option<Arc<str>>,
    /// Authoritative damage for the presented durability bar once a
    /// response states it.
    pub durability_correction: Option<i32>,
}

impl PlayerInventoryLedger {
    /// The authoritative response overlay retained for one player-inventory
    /// slot, or `None` when no accepted correction currently describes it.
    #[must_use]
    pub fn slot_overlay(&self, slot: u8) -> Option<&StackResponseOverlay> {
        self.confirmed.get(Cell::Inventory(slot))?.overlay.as_ref()
    }

    /// The overlay presented for one player-inventory slot: the one travelling
    /// with its predicted stack while a request touches it.
    #[must_use]
    pub fn presented_slot_overlay(&self, slot: u8) -> Option<&StackResponseOverlay> {
        self.view().get(Cell::Inventory(slot))?.overlay.as_ref()
    }

    /// The authoritative response overlay retained for the cursor cell.
    #[must_use]
    pub fn cursor_overlay(&self) -> Option<&StackResponseOverlay> {
        self.confirmed.get(Cell::Cursor)?.overlay.as_ref()
    }

    /// The authoritative response overlay retained for one open generic
    /// storage slot.
    #[must_use]
    pub fn storage_slot_overlay(&self, slot: u8) -> Option<&StackResponseOverlay> {
        self.confirmed.get(Cell::Storage(slot))?.overlay.as_ref()
    }

    /// Resolves responses by request id. A rejection deletes its request's
    /// groups; an acceptance waits for every predecessor before committing.
    pub(super) fn apply_response(&mut self, event: &ItemStackResponseEvent) {
        for response in event.responses.iter() {
            let Some(index) = self.queue.iter().position(|pending| {
                pending.request_id == response.request_id && pending.accepted.is_none()
            }) else {
                continue;
            };
            if response.status == StackResponseStatus::Accepted
                && self.request_is_current(&self.queue[index])
            {
                self.queue[index].accepted = Some(Arc::clone(&response.containers));
            } else {
                self.queue.remove(index);
            }
            self.settle_accepted_heads();
        }
        self.refold();
        // Settlement may have released the last request of a closing window.
        self.finish_closing();
    }
}

/// Merges one accepted correction into a cell's retained overlay, creating
/// the overlay when this is the cell's first corrected response. Only stated
/// fields are written, so a fresh overlay keeps unstated facts absent.
pub(super) fn merge_response_overlay(
    entry: &mut Option<StackResponseOverlay>,
    correction: &StackResponseSlot,
) {
    let overlay = entry.get_or_insert_with(Default::default);
    if !correction.custom_name.is_empty() {
        overlay.custom_name = Some(Arc::clone(&correction.custom_name));
    }
    if !correction.filtered_custom_name.is_empty() {
        overlay.filtered_custom_name = Some(Arc::clone(&correction.filtered_custom_name));
    }
    if correction.durability_correction > 0 {
        overlay.durability_correction = Some(correction.durability_correction);
    }
}
