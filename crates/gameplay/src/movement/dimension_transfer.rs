use std::time::Duration;

use protocol::{ChangeDimensionEvent, Packet};

use super::MovementTicker;

const SERVER_ACK_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug, Clone)]
pub(super) struct DimensionTransfer {
    loading_screen_id: Option<u32>,
    started_at: Duration,
    server_ack: bool,
    loading_started: bool,
    local_ack_sent: bool,
}

impl MovementTicker {
    /// Freezes prediction until the destination area and dimension handshake are complete.
    pub fn begin_dimension_transfer(&mut self, change: ChangeDimensionEvent, now: Duration) {
        self.dimension_transfer = Some(DimensionTransfer {
            loading_screen_id: change.loading_screen_id,
            started_at: now,
            server_ack: false,
            loading_started: false,
            local_ack_sent: false,
        });
    }

    pub fn dimension_transfer_position(&self) -> Option<[f32; 3]> {
        self.dimension_transfer
            .as_ref()
            .map(|_| self.previous_position)
    }

    /// The server's dimension acknowledgement belongs to the session, including sentinel IDs.
    pub fn note_dimension_change_ack(&mut self) {
        if let Some(pending) = self.dimension_transfer.as_mut() {
            pending.server_ack = true;
        }
    }

    /// Retries each handshake write without repeating packets already admitted to transport.
    pub fn flush_dimension_transfer<E>(
        &mut self,
        now: Duration,
        terrain_ready: bool,
        runtime_id: u64,
        mut send: impl FnMut(Packet) -> Result<(), E>,
    ) -> Result<(), E> {
        let Some(pending) = self.dimension_transfer.as_mut() else {
            return Ok(());
        };
        if !pending.loading_started {
            send(protocol::dimension_loading_screen_packet(
                pending.loading_screen_id,
                true,
            ))?;
            pending.loading_started = true;
            return Ok(());
        }
        if !pending.server_ack {
            if now.saturating_sub(pending.started_at) > SERVER_ACK_TIMEOUT {
                pending.server_ack = true;
            }
            return Ok(());
        }
        if !terrain_ready {
            return Ok(());
        }
        if !pending.local_ack_sent {
            send(protocol::dimension_change_ack_packet(runtime_id))?;
            pending.local_ack_sent = true;
        }
        send(protocol::dimension_loading_screen_packet(
            pending.loading_screen_id,
            false,
        ))?;
        self.dimension_transfer = None;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pending() -> MovementTicker {
        let mut movement = MovementTicker::default();
        movement.reset(7, 0, [0.0, 4000.0, 0.0]);
        movement.set_source(super::super::MovementSource::Physics);
        movement.begin_dimension_transfer(
            ChangeDimensionEvent {
                loading_screen_id: Some(42),
                ..Default::default()
            },
            Duration::from_secs(1),
        );
        movement
    }

    fn record(packets: &mut Vec<Packet>) -> impl FnMut(Packet) -> Result<(), ()> + '_ {
        |packet| {
            packets.push(packet);
            Ok(())
        }
    }

    #[test]
    fn transfer_readiness_follows_an_accepted_server_reanchor() {
        let mut movement = pending();
        movement.reanchor_surface_spawn(7, [240.5, 82.0, -17.25]);
        assert_eq!(
            movement.dimension_transfer_position(),
            Some([240.5, 82.0, -17.25])
        );
        assert!(!movement.can_advance_physics_frame());
        let mut packets = Vec::new();
        movement.note_dimension_change_ack();
        for _ in 0..2 {
            movement
                .flush_dimension_transfer(Duration::from_secs(2), true, 5, record(&mut packets))
                .unwrap();
        }
        let session = protocol::BedrockSession { shield_item_id: 0 };
        assert_eq!(
            protocol::encode(&packets[0], &session).unwrap(),
            protocol::encode(
                &protocol::dimension_loading_screen_packet(Some(42), true),
                &session
            )
            .unwrap()
        );
    }

    #[test]
    fn transfer_waits_for_server_ack_and_terrain_then_resumes_once() {
        let mut movement = pending();
        let mut packets = Vec::new();
        assert!(!movement.can_advance_physics_frame());
        movement
            .flush_dimension_transfer(Duration::from_secs(2), true, 5, record(&mut packets))
            .unwrap();
        assert_eq!(packets.len(), 1);
        movement.note_dimension_change_ack();
        movement
            .flush_dimension_transfer(Duration::from_secs(2), false, 5, record(&mut packets))
            .unwrap();
        assert_eq!(packets.len(), 1);
        movement
            .flush_dimension_transfer(Duration::from_secs(2), true, 5, record(&mut packets))
            .unwrap();
        assert_eq!(packets.len(), 3);
        let session = protocol::BedrockSession { shield_item_id: 0 };
        for (actual, expected) in packets.iter().zip([
            protocol::dimension_loading_screen_packet(Some(42), true),
            protocol::dimension_change_ack_packet(5),
            protocol::dimension_loading_screen_packet(Some(42), false),
        ]) {
            assert_eq!(
                protocol::encode(actual, &session).unwrap(),
                protocol::encode(&expected, &session).unwrap()
            );
        }
        assert!(movement.can_advance_physics_frame());
        movement
            .flush_dimension_transfer(Duration::from_secs(3), true, 5, record(&mut packets))
            .unwrap();
        assert_eq!(packets.len(), 3);
    }

    #[test]
    fn timed_out_server_ack_still_requires_the_destination_area() {
        let mut movement = pending();
        let mut packets = Vec::new();
        movement
            .flush_dimension_transfer(Duration::from_secs(11), true, 5, record(&mut packets))
            .unwrap();
        assert_eq!(packets.len(), 1);
        movement
            .flush_dimension_transfer(Duration::from_secs(12), false, 5, record(&mut packets))
            .unwrap();
        assert_eq!(packets.len(), 1);
        movement
            .flush_dimension_transfer(Duration::from_secs(12), true, 5, record(&mut packets))
            .unwrap();
        assert_eq!(packets.len(), 3);
    }

    #[test]
    fn full_transport_retries_without_duplicate_ack_and_session_reset_clears_it() {
        let mut movement = pending();
        movement.note_dimension_change_ack();
        let mut admitted = Vec::new();
        for capacity in [0, 1, 0, 1, 1] {
            let mut remaining = capacity;
            let _ = movement.flush_dimension_transfer(Duration::from_secs(2), true, 5, |packet| {
                if remaining == 0 {
                    return Err(());
                }
                remaining -= 1;
                admitted.push(packet);
                Ok(())
            });
        }
        assert_eq!(admitted.len(), 3);
        assert!(movement.can_advance_physics_frame());
        let mut movement = pending();
        movement.reset(8, 0, [0.0; 3]);
        movement
            .flush_dimension_transfer(Duration::from_secs(20), true, 5, record(&mut admitted))
            .unwrap();
        assert_eq!(admitted.len(), 3);
    }
}
