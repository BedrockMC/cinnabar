use super::{NetworkHandle, Packet, PacketSendError};

impl NetworkHandle {
    #[cfg(test)]
    pub fn send_packet(&self, packet: Packet) -> Result<(), PacketSendError> {
        self.send_packet_with_confirmation(packet, None, None, None, None, None)
    }

    pub(crate) fn send_form_packet(
        &self,
        session: u64,
        packet: Packet,
    ) -> Result<(), PacketSendError> {
        if session == 0 || session != self.session_generation {
            return Err(PacketSendError::Closed(packet));
        }
        self.send_packet_with_confirmation(packet, None, None, None, None, None)
    }
}
