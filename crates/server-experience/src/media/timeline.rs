//! Revisioned server controls; future starts never take effect early.

use super::MAX_DURATION_US;
use crate::runtime::Principal;
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Surface {
    Ui {
        widget: String,
    },
    Quad {
        object: u32,
        generation: u64,
    },
    Entity {
        runtime_id: u64,
        generation: u64,
        material_slot: String,
    },
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Operation {
    Prepare { media_id: String },
    Play { position_us: u64 },
    Pause { position_us: u64 },
    Seek { position_us: u64 },
    SetLoop { bounds_us: Option<[u64; 2]> },
    SetVolume { per_mille: u16 },
    Attach { surface: Surface },
    Detach,
    Stop,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Message {
    pub owner: Principal,
    pub instance: u32,
    pub generation: u64,
    pub timeline: String,
    pub world_epoch: u64,
    pub revision: u64,
    pub effective_server_us: u64,
    pub operation: Operation,
}

#[derive(Clone, Debug, Default)]
pub struct Playback {
    pub playing: bool,
    pub stopped: bool,
    pub media_id: Option<String>,
    pub position_us: u64,
    pub anchor_us: u64,
    pub loop_us: Option<[u64; 2]>,
    pub volume: u16,
    pub surface: Option<Surface>,
    pub decode_generation: u64,
    revision: u64,
    last_effective_us: Option<u64>,
    pending: VecDeque<Message>,
}

impl Playback {
    /// Rejects stale routes, revisions and unbounded scheduling before mutating state.
    pub fn enqueue(
        &mut self,
        message: Message,
        owner: &Principal,
        epoch: u64,
        timeline: &str,
        now_us: u64,
    ) -> Result<()> {
        ensure!(
            &message.owner == owner
                && message.world_epoch == epoch
                && message.timeline == timeline
                && message.instance == 1
                && message.generation == 1,
            "stale media route"
        );
        ensure!(
            message.revision > self.revision && self.pending.len() < 32,
            "media revision or queue rejected"
        );
        ensure!(
            message.effective_server_us <= now_us.saturating_add(30_000_000),
            "media schedule too distant"
        );
        ensure!(
            self.last_effective_us
                .is_none_or(|last| last <= message.effective_server_us),
            "media controls reordered"
        );
        match &message.operation {
            Operation::Play { position_us }
            | Operation::Pause { position_us }
            | Operation::Seek { position_us } => {
                ensure!(*position_us <= MAX_DURATION_US, "media position too large")
            }
            Operation::SetLoop {
                bounds_us: Some([start, end]),
            } => ensure!(start < end && *end <= MAX_DURATION_US, "invalid loop range"),
            Operation::SetVolume { per_mille } => ensure!(*per_mille <= 1000, "invalid volume"),
            _ => {}
        }
        self.revision = message.revision;
        self.last_effective_us = Some(message.effective_server_us);
        self.pending.push_back(message);
        Ok(())
    }

    /// Applies due controls at their authored time, preserving the common timeline.
    pub fn advance(&mut self, server_us: u64, duration_us: u64) -> Result<()> {
        while self
            .pending
            .front()
            .is_some_and(|message| message.effective_server_us <= server_us)
        {
            let message = self.pending.pop_front().expect("front checked");
            self.position_us = self.position(message.effective_server_us, duration_us);
            self.anchor_us = message.effective_server_us;
            match message.operation {
                Operation::Prepare { media_id } => {
                    self.media_id = Some(media_id);
                    self.playing = false;
                    self.stopped = false;
                    self.reset_decode()?;
                }
                Operation::Play { position_us } => {
                    self.position_us = position_us.min(duration_us);
                    self.playing = true;
                    self.stopped = false;
                    self.reset_decode()?;
                }
                Operation::Pause { position_us } => {
                    self.position_us = position_us.min(duration_us);
                    self.playing = false;
                }
                Operation::Seek { position_us } => {
                    self.position_us = position_us.min(duration_us);
                    self.reset_decode()?;
                }
                Operation::SetLoop { bounds_us } => {
                    ensure!(
                        bounds_us.is_none_or(|[_, end]| end <= duration_us),
                        "loop exceeds duration"
                    );
                    self.loop_us = bounds_us;
                }
                Operation::SetVolume { per_mille } => self.volume = per_mille,
                Operation::Attach { surface } => self.surface = Some(surface),
                Operation::Detach => self.surface = None,
                Operation::Stop => {
                    self.playing = false;
                    self.stopped = true;
                    self.surface = None;
                    self.reset_decode()?;
                }
            }
        }
        Ok(())
    }

    /// Calculates desired position with checked integer arithmetic and loop wrapping.
    pub fn position(&self, server_us: u64, duration_us: u64) -> u64 {
        let position = self.position_us.saturating_add(if self.playing {
            server_us.saturating_sub(self.anchor_us)
        } else {
            0
        });
        if let Some([start, end]) = self.loop_us
            && position >= end
        {
            start + (position - start) % (end - start)
        } else {
            position.min(duration_us)
        }
    }

    /// Invalidates queued PCM, frames and range reads on discontinuity.
    fn reset_decode(&mut self) -> Result<()> {
        self.decode_generation = self
            .decode_generation
            .checked_add(1)
            .ok_or_else(|| anyhow::anyhow!("media generation exhausted"))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Builds one controlled future command without a network or decoder.
    fn message(owner: &Principal, revision: u64, at: u64, operation: Operation) -> Message {
        Message {
            owner: owner.clone(),
            instance: 1,
            generation: 1,
            timeline: "cinema".into(),
            world_epoch: 1,
            revision,
            effective_server_us: at,
            operation,
        }
    }

    #[test]
    fn applied_controls_keep_the_last_accepted_timestamp() {
        let owner = Principal {
            session: "session".into(),
            bundle: "cinema".into(),
            generation: 1,
        };
        let mut playback = Playback::default();
        playback
            .enqueue(
                message(&owner, 1, 100, Operation::Play { position_us: 10 }),
                &owner,
                1,
                "cinema",
                100,
            )
            .unwrap();
        playback.advance(200, 1000).unwrap();
        assert!(playback.pending.is_empty());
        assert!(
            playback
                .enqueue(
                    message(&owner, 2, 99, Operation::SetVolume { per_mille: 500 }),
                    &owner,
                    1,
                    "cinema",
                    200
                )
                .is_err()
        );
        assert_eq!(playback.position(200, 1000), 110);
        playback
            .enqueue(
                message(&owner, 2, 100, Operation::SetVolume { per_mille: 500 }),
                &owner,
                1,
                "cinema",
                200,
            )
            .unwrap();
        playback.advance(200, 1000).unwrap();
        assert_eq!(playback.position(200, 1000), 110);
        assert_eq!(playback.volume, 500);
    }

    #[test]
    fn scheduled_play_waits_and_seek_revokes_old_decoder_output() {
        let owner = Principal {
            session: "session".into(),
            bundle: "cinema".into(),
            generation: 1,
        };
        let mut playback = Playback::default();
        playback
            .enqueue(
                message(&owner, 1, 1_000_000, Operation::Play { position_us: 0 }),
                &owner,
                1,
                "cinema",
                0,
            )
            .unwrap();
        playback.advance(999_999, 10_000_000).unwrap();
        assert!(!playback.playing);
        playback.advance(1_250_000, 10_000_000).unwrap();
        assert_eq!(playback.position(1_250_000, 10_000_000), 250_000);
        let previous = playback.decode_generation;
        playback
            .enqueue(
                message(
                    &owner,
                    2,
                    1_250_000,
                    Operation::Seek {
                        position_us: 5_000_000,
                    },
                ),
                &owner,
                1,
                "cinema",
                1_250_000,
            )
            .unwrap();
        playback.advance(1_250_000, 10_000_000).unwrap();
        assert!(playback.decode_generation > previous);
        assert_eq!(playback.position(1_250_000, 10_000_000), 5_000_000);
        assert!(
            playback
                .enqueue(
                    message(&owner, 2, 1_250_000, Operation::Stop),
                    &owner,
                    1,
                    "cinema",
                    1_250_000
                )
                .is_err()
        );
        assert!(
            playback
                .enqueue(
                    message(&owner, 3, 1_250_000, Operation::Stop),
                    &owner,
                    2,
                    "cinema",
                    1_250_000
                )
                .is_err()
        );
    }
}
