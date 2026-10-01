//! Trusted session controller; remote data never supplies consent controls.

mod driver;
mod live;

use std::sync::Arc;
use server_experience::session::Session;

#[derive(Clone, Debug, Default)]
pub(crate) struct ExperienceSession {
    pub(crate) audience: Option<String>,
    pub(crate) marker: Option<Arc<[u8]>>,
    pub(crate) session: Session,
    pub(crate) handled_marker: bool,
    pub(crate) active: bool,
    pub(crate) epoch: u64,
    pub(crate) incoming: std::collections::VecDeque<(u64, Vec<u8>)>,
    incoming_bytes: usize,
}

impl ExperienceSession {
    /// Ends every permission while retaining the host-selected destination.
    pub(crate) fn reset(&mut self) {
        self.marker = None;
        self.session = Session::default();
        self.handled_marker = false;
        self.active = false;
        self.incoming.clear();
        self.incoming_bytes = 0;
    }

    /// Binds a host-selected address, never an address inside a pack or packet.
    pub(crate) fn select_destination(&mut self, address: &str) {
        self.audience = server_experience::negotiation::canonical_audience(address).ok();
    }

    /// Consumes only control messages that crossed the world publication barrier.
    pub(crate) fn receive(&mut self, bytes: &[u8], now_ms: u64) {
        if self.active {
            if self.incoming.len() >= server_experience::policy::MAX_QUEUE_MESSAGES
                || bytes.len() > server_experience::policy::MAX_QUEUE_BYTES - self.incoming_bytes
            {
                self.session.disable();
                self.active = false;
                self.incoming.clear();
                self.incoming_bytes = 0;
            } else {
                self.incoming_bytes += bytes.len();
                self.incoming.push_back((now_ms, bytes.to_vec()));
            }
            return;
        }
        if let Err(error) = self.session.receive(bytes, unix_seconds(), now_ms) {
            bevy::log::warn!(%error, "server experience control rejected");
        }
    }

    /// Releases bounded ingress accounting when the runtime consumes a message.
    pub(crate) fn pop(&mut self) -> Option<(u64, Vec<u8>)> {
        let entry = self.incoming.pop_front()?;
        self.incoming_bytes -= entry.1.len();
        Some(entry)
    }
}

/// Uses wall time only for signed expiration, never for media presentation.
pub(crate) fn unix_seconds() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)
        .map_or(u64::MAX, |time| time.as_secs())
}

pub(crate) use driver::configure;
