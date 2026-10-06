//! Sends the player's server trust answers to the core off the frame thread.

use std::path::PathBuf;
use std::thread;

use protocol::launcher_control::answer_server_trust;

/// Answers trust prompt `id`; a lost answer leaves the prompt to end with its join.
pub(super) fn send(socket_dir: PathBuf, id: u64, trusted: bool) {
    thread::spawn(move || {
        let Some(runtime) = super::runtime() else {
            return;
        };
        if let Err(error) = runtime.block_on(answer_server_trust(&socket_dir, id, trusted)) {
            bevy::log::warn!(%error, "server trust answer was not delivered");
        }
    });
}
