//! Private core control; this never exposes packet contents to a component.

use crate::BridgeError;
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Deserialize)]
pub struct PacketDelayLease {
    pub delay_ms: u32,
    pub lease_ms: u32,
}

#[derive(Serialize)]
struct Request {
    delay_ms: u32,
}

pub async fn set_packet_delay(
    socket_dir: &Path,
    delay_ms: u32,
) -> Result<PacketDelayLease, BridgeError> {
    crate::account::call(socket_dir, "packet_delay.v1", Some(Request { delay_ms })).await
}
