use std::time::Duration;

use client_world::WorldStream;

use crate::{movement::MovementTicker, runtime::network::NetworkHandle};

pub(super) fn flush_dimension_transfer(
    movement: &mut MovementTicker,
    stream: &WorldStream,
    network: Option<&NetworkHandle>,
    now: Duration,
) {
    let Some(position) = movement.dimension_transfer_position() else {
        return;
    };
    let Some(network) = network else {
        return;
    };
    let ready = stream.dimension_transfer_area_ready(position);
    let _ =
        movement.flush_dimension_transfer(now, ready, stream.local_player_runtime_id(), |packet| {
            network.send_movement_packet(packet)
        });
}
