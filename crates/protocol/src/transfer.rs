pub use jolyne::transfer::{MAX_TRANSFER_HOST_BYTES, ServerTransferEvent, ServerTransferRejection};

impl crate::ProtocolError {
    /// Returns the validated destination when a Transfer ends any login phase.
    pub fn server_transfer(&self) -> Option<ServerTransferEvent> {
        match self {
            Self::Session(jolyne::error::JolyneError::Protocol(
                jolyne::error::ProtocolError::ServerTransfer(target),
            )) => Some(target.clone()),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use valentine::bedrock::version::v1_26_51::{McpePacketData, TransferPacket};

    fn transfer_data(address: &str, port: u16) -> McpePacketData {
        McpePacketData::TransferPacket(Box::new(TransferPacket {
            server_address: address.to_owned(),
            server_port: port,
            reload_world: false,
            ..Default::default()
        }))
    }

    #[test]
    fn normalization_retains_a_bounded_target() {
        let event =
            ServerTransferEvent::from_packet_data(&transfer_data("play.example.net", 19133))
                .expect("well-formed target")
                .expect("transfer packet normalizes");
        assert_eq!(event.host, "play.example.net");
        assert_eq!(event.port, 19133);
        assert!(!event.reload_world);
    }

    #[test]
    fn normalization_retains_the_reload_flag_and_trims_surrounding_whitespace() {
        let data = McpePacketData::TransferPacket(Box::new(TransferPacket {
            server_address: " game.example.net ".to_owned(),
            server_port: 19133,
            reload_world: true,
            ..Default::default()
        }));
        let event = ServerTransferEvent::from_packet_data(&data)
            .expect("well-formed target")
            .expect("transfer packet normalizes");
        assert_eq!(event.host, "game.example.net");
        assert!(event.reload_world);
    }

    #[test]
    fn normalization_ignores_non_transfer_packets() {
        let other =
            McpePacketData::SetTimePacket(valentine::bedrock::version::v1_26_51::SetTimePacket {
                time: 7,
            });
        assert_eq!(
            ServerTransferEvent::from_packet_data(&other).expect("non-transfer normalizes"),
            None
        );
    }

    #[test]
    fn empty_and_whitespace_only_hosts_are_semantic_rejections() {
        assert_eq!(
            ServerTransferEvent::from_packet_data(&transfer_data("", 19133)),
            Err(ServerTransferRejection::EmptyHost)
        );
        assert_eq!(
            ServerTransferEvent::from_packet_data(&transfer_data("   ", 19133)),
            Err(ServerTransferRejection::EmptyHost)
        );
    }

    #[test]
    fn oversize_hosts_are_semantic_rejections_without_truncation() {
        let bytes = "a".repeat(MAX_TRANSFER_HOST_BYTES + 1);
        match ServerTransferEvent::from_packet_data(&transfer_data(&bytes, 19133)) {
            Err(ServerTransferRejection::HostTooLong { bytes }) => {
                assert_eq!(bytes, MAX_TRANSFER_HOST_BYTES + 1);
            }
            other => panic!("oversize host must be rejected, got {other:?}"),
        }
        let exact = "a".repeat(MAX_TRANSFER_HOST_BYTES);
        assert!(
            ServerTransferEvent::from_packet_data(&transfer_data(&exact, 19133))
                .expect("exact bound is well-formed")
                .is_some()
        );
    }

    #[test]
    fn control_characters_are_semantic_rejections() {
        assert_eq!(
            ServerTransferEvent::from_packet_data(&transfer_data("play.exam\x07ple.net", 19133)),
            Err(ServerTransferRejection::InvalidHostCharacter)
        );
        assert_eq!(
            ServerTransferEvent::from_packet_data(&transfer_data("play exam ple.net", 19133)),
            Err(ServerTransferRejection::InvalidHostCharacter)
        );
    }

    #[test]
    fn zero_port_is_a_semantic_rejection() {
        assert_eq!(
            ServerTransferEvent::from_packet_data(&transfer_data("play.example.net", 0)),
            Err(ServerTransferRejection::ZeroPort)
        );
    }

    #[test]
    fn cross_host_targets_are_not_filtered_by_any_allowlist() {
        // A vanilla lobby routinely hops clients to an unrelated minigame
        // host. Well-formedness must not reject the shape of that target.
        let event = ServerTransferEvent::from_packet_data(&transfer_data(
            "minigames.other-host.example",
            19321,
        ))
        .expect("well-formed target")
        .expect("cross-host transfer normalizes");
        assert_eq!(event.host, "minigames.other-host.example");
    }
}
