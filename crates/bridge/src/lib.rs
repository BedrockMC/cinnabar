//! Local stream bridge between the Rust client and Go core.

mod account;
mod endpoint;
mod error;
mod framed;
mod status;
mod worlds;

use std::path::Path;

pub use account::{
    Account, AuthState, ConnectTarget, Events, Friend, Realm, ServerDisconnect, account_status,
    connect_target, list_friends, list_realms, poll_events, sign_out,
};
pub use error::BridgeError;
pub use framed::FramedStream;
pub use status::{
    Lifecycle, PackAcquisition, PackAdmission, PackApplication, PackDownstreamOutcome, PackOffer,
    StatusV1, TransferPending, read_status, report_pack_application,
};
pub use worlds::{
    Difficulty, GameMode, Generator, NewWorld, World, WorldState, WorldStatus, close_world,
    create_world, delete_world, list_worlds, open_world, rename_world, set_world_paused,
    world_status,
};

/// Returns the platform endpoint used for the logical socket directory.
#[must_use]
pub fn endpoint_path(socket_dir: &Path) -> std::path::PathBuf {
    endpoint::endpoint_path(socket_dir, endpoint::EndpointKind::Game)
}

/// Returns the platform control endpoint used for the logical socket directory.
#[must_use]
pub fn control_endpoint_path(socket_dir: &Path) -> std::path::PathBuf {
    endpoint::endpoint_path(socket_dir, endpoint::EndpointKind::Control)
}

/// Largest payload accepted by the local bridge framing protocol.
pub const MAX_FRAME_LEN: usize = 64 * 1024 * 1024;

/// Connects to the local Go core endpoint published in `socket_dir`.
pub async fn connect(socket_dir: &Path) -> anyhow::Result<FramedStream> {
    let stream = endpoint::connect(socket_dir, endpoint::EndpointKind::Game).await?;
    Ok(FramedStream::new(stream))
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use bytes::Bytes;
    use futures::{Sink, Stream};

    use super::{BridgeError, FramedStream, connect};

    fn assert_transport<T>()
    where
        T: Stream<Item = Result<Bytes, BridgeError>>
            + Sink<Bytes, Error = BridgeError>
            + Unpin
            + Send,
    {
    }

    #[test]
    fn public_transport_contract_is_stable() {
        assert_transport::<FramedStream>();
        let _ = connect;
    }

    #[tokio::test]
    async fn connect_preserves_bridge_error_in_anyhow_result() {
        let error = match connect(Path::new("")).await {
            Ok(_) => panic!("empty socket directory must fail"),
            Err(error) => error,
        };

        assert!(matches!(
            error.downcast_ref::<BridgeError>(),
            Some(BridgeError::InvalidEndpoint { .. })
        ));
    }
}
