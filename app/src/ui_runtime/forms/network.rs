use super::{FormTransportError, flush_form_response};
use crate::{
    runtime::network::{NetworkHandle, PacketSendError},
    ui_runtime::UiRuntime,
};
use bevy::prelude::{Res, ResMut};

pub(crate) fn flush_server_form_network(
    mut runtime: ResMut<UiRuntime>,
    network: Res<NetworkHandle>,
) {
    let session = runtime.session_id();
    // Terminal controls retire the session before another enqueue attempt.
    if network.closed_command_has_pending_control() {
        return;
    }
    let _ = flush_form_response(&mut runtime, |packet| {
        network
            .send_form_packet(session, packet)
            .map_err(|error| match error {
                PacketSendError::Full(_) => FormTransportError::Full,
                PacketSendError::Closed(_) => FormTransportError::Closed,
            })
    });
}
