//! Private helper boundary shared by supervision and deterministic scheduling tests.

use anyhow::Result;
use mod_host::helper::{Dispatch, Helper};
use server_experience::runtime::{Capabilities, Principal, Transaction};
use std::path::Path;

pub(super) trait Worker: Sized {
    /// Starts a helper whose first response is its initialization transaction.
    fn spawn(
        executable: &Path,
        bytes: &[u8],
        owner: Principal,
        capabilities: Capabilities,
        epoch: u64,
    ) -> Result<Self>;
    /// Returns a completed callback without waiting on the render thread.
    fn poll(&mut self) -> Option<Result<Transaction>>;
    /// Submits one callback to an idle helper.
    fn dispatch(&mut self, request: Dispatch) -> Result<()>;
}

impl Worker for Helper {
    /// Starts the developer helper with the host-selected capabilities.
    fn spawn(
        executable: &Path,
        bytes: &[u8],
        owner: Principal,
        capabilities: Capabilities,
        epoch: u64,
    ) -> Result<Self> {
        Self::spawn_developer(executable, bytes, owner, capabilities, epoch)
    }

    /// Polls the supervised process without blocking the frame.
    fn poll(&mut self) -> Option<Result<Transaction>> {
        self.poll()
    }

    /// Forwards an event only after supervision has admitted its callback.
    fn dispatch(&mut self, request: Dispatch) -> Result<()> {
        self.dispatch(request)
    }
}
