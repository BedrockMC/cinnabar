use bevy::prelude::Message;
use client_world::WorldStream;

/// App-facing audio transport seam. Playback and sound resolution intentionally
/// live downstream of this packet-preserving ingress message.
#[derive(Debug, Clone, PartialEq, Message)]
pub(crate) struct SequencedAudioEvent {
    /// Immutable local WorldStream lifetime, not a sampled clock or account identity.
    pub(crate) origin_stream_session_id: u64,
    pub(crate) sequence: u64,
    pub(crate) event: protocol::AudioEvent,
}

pub(crate) fn drain_committed_audio(
    stream: &mut WorldStream,
    mut forward: impl FnMut(SequencedAudioEvent),
) {
    let origin_stream_session_id = stream.actor_session_id();
    for committed in stream.take_committed_audio() {
        forward(SequencedAudioEvent {
            origin_stream_session_id,
            sequence: committed.sequence,
            event: committed.event,
        });
    }
}
