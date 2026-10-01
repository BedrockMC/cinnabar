use super::{BatchSendError, NetworkCommand, NetworkHandle};

impl NetworkHandle {
    /// Whether a committed echo still awaits command FIFO capacity.
    pub(crate) fn has_pending_latency_reply(&self) -> bool {
        self.pending_latency_reply
            .lock()
            .expect("latency reply lock")
            .is_some()
    }

    /// Retains one committed echo until the outbound FIFO has room.
    pub(crate) fn send_latency_reply(&self, creation_time: u64) -> Result<(), BatchSendError> {
        self.flush_latency_reply()?;
        *self
            .pending_latency_reply
            .lock()
            .expect("latency reply lock") =
            Some(protocol::network_stack_latency_reply(creation_time));
        match self.flush_latency_reply() {
            Err(BatchSendError::Full) => Ok(()),
            result => result,
        }
    }

    /// Flushes the committed echo before any later outbound packet.
    pub(crate) fn flush_latency_reply(&self) -> Result<(), BatchSendError> {
        let mut pending = self
            .pending_latency_reply
            .lock()
            .expect("latency reply lock");
        if pending.is_none() {
            return Ok(());
        }
        let permit = match self.commands.try_reserve() {
            Ok(permit) => permit,
            Err(tokio::sync::mpsc::error::TrySendError::Full(())) => {
                return Err(BatchSendError::Full);
            }
            Err(tokio::sync::mpsc::error::TrySendError::Closed(())) => {
                *pending = None;
                return Err(BatchSendError::Closed);
            }
        };
        permit.send(NetworkCommand::Send {
            packet: pending.take().expect("pending latency reply"),
            sub_chunk: None,
            chat: None,
            physics: None,
            physics_reanchor: None,
            interaction: None,
        });
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::network::session::PacketSendError;
    use tokio::sync::mpsc;

    /// Creates one observable outbound FIFO slot.
    fn handle() -> (NetworkHandle, mpsc::Receiver<NetworkCommand>) {
        let (mut handle, _) = NetworkHandle::stub();
        let (commands, receiver) = mpsc::channel(1);
        handle.commands = commands;
        (handle, receiver)
    }

    /// Checks the next packet's bytes against the expected probe identity.
    fn assert_probe(receiver: &mut mpsc::Receiver<NetworkCommand>, timestamp: u64) {
        let NetworkCommand::Send { packet, .. } = receiver.try_recv().unwrap();
        let session = protocol::BedrockSession { shield_item_id: 0 };
        assert_eq!(
            protocol::encode(&packet, &session).unwrap(),
            protocol::encode(&protocol::network_stack_latency_reply(timestamp), &session).unwrap(),
        );
    }

    #[test]
    fn two_latency_fences_and_later_packets_keep_fifo_order_under_backpressure() {
        let (handle, mut receiver) = handle();
        handle
            .send_packet(protocol::network_stack_latency_reply(1))
            .unwrap();
        handle.send_latency_reply(2).unwrap();
        assert_eq!(handle.pending_command_count(), 2);
        assert!(matches!(
            handle.send_latency_reply(3),
            Err(BatchSendError::Full)
        ));
        assert!(matches!(
            handle.send_packet(protocol::network_stack_latency_reply(4)),
            Err(PacketSendError::Full(_))
        ));
        assert_probe(&mut receiver, 1);
        handle.send_latency_reply(3).unwrap();
        assert_probe(&mut receiver, 2);
        assert!(matches!(
            handle.send_packet(protocol::network_stack_latency_reply(4)),
            Err(PacketSendError::Full(_))
        ));
        assert_probe(&mut receiver, 3);
        handle
            .send_packet(protocol::network_stack_latency_reply(4))
            .unwrap();
        assert_probe(&mut receiver, 4);
        assert_eq!(handle.pending_command_count(), 0);
    }

    #[test]
    fn closing_or_replacing_a_session_drops_the_retained_echo() {
        let (mut handle, receiver) = handle();
        handle
            .send_packet(protocol::network_stack_latency_reply(1))
            .unwrap();
        handle.send_latency_reply(2).unwrap();
        drop(receiver);
        assert!(matches!(
            handle.flush_latency_reply(),
            Err(BatchSendError::Closed)
        ));
        assert!(handle.pending_latency_reply.lock().unwrap().is_none());
        handle.shutdown();
        assert!(
            NetworkHandle::disconnected()
                .pending_latency_reply
                .lock()
                .unwrap()
                .is_none()
        );
    }
}
