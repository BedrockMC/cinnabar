//! Trusted session controller; remote data never supplies consent controls.

mod driver;

use std::sync::Arc;
use server_experience::session::Session;

#[derive(Clone, Debug, Default)]
pub(crate) struct ExperienceSession {
    pub(crate) audience: Option<String>,
    pub(crate) marker: Option<Arc<[u8]>>,
    pub(crate) session: Session,
    pub(crate) handled_marker: bool,
}

impl ExperienceSession {
    /// Ends every permission while retaining the host-selected destination.
    pub(crate) fn reset(&mut self) {
        self.marker = None;
        self.session = Session::default();
        self.handled_marker = false;
    }

    /// Binds a host-selected address, never an address inside a pack or packet.
    pub(crate) fn select_destination(&mut self, address: &str) {
        self.audience = server_experience::negotiation::canonical_audience(address).ok();
    }

    /// Consumes only control messages that crossed the world publication barrier.
    pub(crate) fn receive(&mut self, bytes: &[u8], now_ms: u64) {
        if let Err(error) = self.session.receive(bytes, unix_seconds(), now_ms) {
            bevy::log::warn!(%error, "server experience control rejected");
        }
    }
}

/// Uses wall time only for signed expiration, never for media presentation.
pub(crate) fn unix_seconds() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)
        .map_or(u64::MAX, |time| time.as_secs())
}

pub(crate) use driver::configure;
