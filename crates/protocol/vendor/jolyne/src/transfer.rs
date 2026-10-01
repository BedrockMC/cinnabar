use crate::valentine::McpePacketData;

/// Longest retained transfer host, in bytes.
///
/// DNS names cap at 253 octets and literal IPv6 at 45 characters, so this
/// bounds every legitimate target while refusing unbounded wire strings.
pub const MAX_TRANSFER_HOST_BYTES: usize = 255;

/// Bounded, vendor-neutral record of a server-directed transfer target.
///
/// The host is trimmed of surrounding whitespace and validated for
/// well-formedness only; following the target is a policy decision owned above
/// the protocol layer. Vanilla servers legitimately transfer across unrelated
/// hosts, so no address allowlist exists here. The optional gatherings
/// configuration is deliberately not retained: no production consumer exists
/// and the field is absent from ordinary retail transfers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerTransferEvent {
    pub host: String,
    pub port: u16,
    pub reload_world: bool,
}

/// Why a completely decoded Transfer packet named an unusable target.
///
/// These are semantic oddities, not wire failures: the packet decoded within
/// every length bound, so the session survives and the caller counts the
/// rejection instead of tearing down.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServerTransferRejection {
    EmptyHost,
    HostTooLong { bytes: usize },
    InvalidHostCharacter,
    ZeroPort,
}

impl ServerTransferEvent {
    /// Normalizes one decoded packet into a bounded transfer target.
    ///
    /// Non-transfer packets normalize to nothing. Well-formed wire naming an
    /// unusable target (empty or oversized host, forbidden characters, zero
    /// port) returns the counted [`ServerTransferRejection`] instead of an
    /// event; truncated or otherwise malformed wire never reaches here because
    /// decode failures stay fatal upstream.
    pub fn from_packet_data(
        data: &McpePacketData,
    ) -> Result<Option<Self>, ServerTransferRejection> {
        let McpePacketData::TransferPacket(packet) = data else {
            return Ok(None);
        };
        Self::from_wire_fields(
            &packet.server_address,
            packet.server_port,
            packet.reload_world,
        )
    }

    /// Validates a decoded destination before retaining it for the reconnect owner.
    fn from_wire_fields(
        address: &str,
        port: u16,
        reload_world: bool,
    ) -> Result<Option<Self>, ServerTransferRejection> {
        let host = address.trim();
        if host.is_empty() {
            return Err(ServerTransferRejection::EmptyHost);
        }
        if host.len() > MAX_TRANSFER_HOST_BYTES {
            return Err(ServerTransferRejection::HostTooLong { bytes: host.len() });
        }
        if host.chars().any(is_forbidden_host_character) {
            return Err(ServerTransferRejection::InvalidHostCharacter);
        }
        if port == 0 {
            return Err(ServerTransferRejection::ZeroPort);
        }
        Ok(Some(Self {
            host: host.to_owned(),
            port,
            reload_world,
        }))
    }
}

/// Rejects ASCII control characters and interior ASCII whitespace.
///
/// This is a well-formedness floor, not a hostname grammar: unusual but
/// dialable characters are retained and fail visibly when the core dials the
/// target, which keeps the boundary honest without inventing policy.
fn is_forbidden_host_character(character: char) -> bool {
    matches!(character, '\0'..='\x1f' | '\x7f' | ' ')
}
