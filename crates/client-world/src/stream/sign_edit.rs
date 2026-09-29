//! The server's request to open a sign editor, held until the presentation takes it.

use protocol::OpenSignEvent;

use super::WorldStream;

/// One sign face the server asked the player to edit.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SignEditRequest {
    pub position: [i32; 3],
    pub front: bool,
}

impl WorldStream {
    pub(super) fn consume_open_sign(&mut self, event: OpenSignEvent) {
        if event.dimension != self.current_dimension {
            return;
        }
        // A newer request supersedes an untaken one, as a second open would.
        self.pending_sign_edit = Some(SignEditRequest {
            position: event.position,
            front: event.front,
        });
    }

    /// Takes the pending sign-edit request, if any.
    pub fn take_pending_sign_edit(&mut self) -> Option<SignEditRequest> {
        self.pending_sign_edit.take()
    }
}
