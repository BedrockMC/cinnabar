//! Typed, bounded envelopes. Runtime bytes never become arbitrary Bedrock packets.

use std::collections::VecDeque;
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
use crate::{manifest::identifier, negotiation::Grant, policy::*};

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(tag = "type", content = "value", rename_all = "snake_case", deny_unknown_fields)]
pub enum Scalar { Bool(bool), Integer(i64), Text(String), Choice(u16) }

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Field {
    Bool,
    Integer { min: i64, max: i64 },
    Text { max_bytes: u16 },
    Choice { variants: u16 },
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Direction { ToClient, ToServer }

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Channel {
    pub id: String,
    pub schema: u16,
    pub direction: Direction,
    pub fields: Vec<Field>,
}

impl Channel {
    /// Validates the declared positional record before guest dispatch or sending.
    pub fn validate(&self, payload: &[Scalar], direction: Direction) -> Result<()> {
        ensure!(identifier(&self.id) && self.direction == direction && self.fields.len() <= 64, "channel denied");
        ensure!(payload.len() == self.fields.len(), "record field count mismatch");
        for (field, value) in self.fields.iter().zip(payload) {
            let valid = match (field, value) {
                (Field::Bool, Scalar::Bool(_)) => true,
                (Field::Integer { min, max }, Scalar::Integer(value)) => min <= value && value <= max,
                (Field::Text { max_bytes }, Scalar::Text(value)) => value.len() <= usize::from(*max_bytes),
                (Field::Choice { variants }, Scalar::Choice(value)) => value < variants,
                _ => false,
            };
            ensure!(valid, "record field rejected");
        }
        ensure!(serde_json::to_vec(payload)?.len() <= MAX_PAYLOAD_BYTES, "payload too large");
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Envelope {
    pub version: u16,
    pub session: String,
    pub connection: String,
    pub subclient: u8,
    pub bundle: String,
    pub generation: u64,
    pub channel: String,
    pub schema: u16,
    pub sequence: u64,
    pub world_epoch: u64,
    pub payload: Vec<Scalar>,
}

#[derive(Clone, Debug)]
pub struct RateLimit {
    last_ms: u64,
    messages: u64,
    bytes: u64,
}

impl RateLimit {
    /// Gives at most one second of initial burst credit.
    pub fn new(now_ms: u64) -> Self {
        Self { last_ms: now_ms, messages: MAX_MESSAGES_PER_SECOND * 1000, bytes: MAX_BYTES_PER_SECOND * 1000 }
    }

    /// Charges before parsing; a clock reversal cannot mint extra credit.
    pub fn charge(&mut self, size: usize, now_ms: u64) -> Result<()> {
        let elapsed = now_ms.saturating_sub(self.last_ms).min(1000);
        self.last_ms = self.last_ms.max(now_ms);
        self.messages = (self.messages + elapsed * MAX_MESSAGES_PER_SECOND).min(MAX_MESSAGES_PER_SECOND * 1000);
        self.bytes = (self.bytes + elapsed * MAX_BYTES_PER_SECOND).min(MAX_BYTES_PER_SECOND * 1000);
        ensure!(size <= protocol::MAX_EXPERIENCE_ENVELOPE_BYTES, "envelope too large");
        let cost = size as u64 * 1000;
        ensure!(self.messages >= 1000 && self.bytes >= cost, "channel rate exceeded");
        self.messages -= 1000;
        self.bytes -= cost;
        Ok(())
    }
}

/// One reliable direction, shared by every bundle in a server session.
#[derive(Debug)]
pub struct Ingress {
    rate: RateLimit,
    next: u64,
    queue: VecDeque<(u64, usize, Envelope)>,
    bytes: usize,
    failed: bool,
    pub skipped: u64,
}

impl Ingress {
    /// Starts a fresh sequence space after a signed handshake.
    pub fn new(now_ms: u64) -> Self {
        Self { rate: RateLimit::new(now_ms), next: 1, queue: VecDeque::new(), bytes: 0, failed: false, skipped: 0 }
    }

    /// Quarantines the optional channel on replay, a sequence gap or overflow.
    pub fn receive(&mut self, bytes: &[u8], now_ms: u64, publication: u64, grant: &Grant, channels: &[Channel]) -> Result<()> {
        let result = self.receive_inner(bytes, now_ms, publication, grant, channels);
        if result.is_err() {
            self.failed = true;
            self.queue.clear();
            self.bytes = 0;
        }
        result
    }

    /// Validates identity and schema after charging aggregate ingress cost.
    fn receive_inner(&mut self, bytes: &[u8], now_ms: u64, publication: u64, grant: &Grant, channels: &[Channel]) -> Result<()> {
        ensure!(!self.failed, "channel quarantined");
        self.rate.charge(bytes.len(), now_ms)?;
        let message: Envelope = serde_json::from_slice(bytes)?;
        ensure!(message.version == WIRE_VERSION && message.session == grant.session
            && message.connection == grant.connection && message.subclient == grant.subclient, "wrong session route");
        ensure!(message.sequence == self.next, "replay or reliable sequence gap");
        self.next = self.next.checked_add(1).ok_or_else(|| anyhow::anyhow!("sequence exhausted"))?;
        ensure!(message.generation == 1 && grant.offer.offer.packages.iter().any(|p| p.id == message.bundle), "wrong bundle generation");
        ensure!(channels.len() <= MAX_CHANNELS, "channel limit exceeded");
        let Some(channel) = channels.iter().find(|c| c.id == message.channel && c.schema == message.schema) else {
            self.skipped = self.skipped.saturating_add(1);
            return Ok(());
        };
        channel.validate(&message.payload, Direction::ToClient)?;
        ensure!(self.queue.len() < MAX_QUEUE_MESSAGES && bytes.len() <= MAX_QUEUE_BYTES - self.bytes, "reliable queue overflow");
        self.bytes += bytes.len();
        self.queue.push_back((publication, bytes.len(), message));
        Ok(())
    }

    /// Publishes only after preceding world events; old dimension work is discarded.
    pub fn pop(&mut self, committed: u64, world_epoch: u64) -> Option<Envelope> {
        loop {
            if self.queue.front()?.0 > committed { return None; }
            let (_, bytes, message) = self.queue.pop_front()?;
            self.bytes -= bytes;
            if message.world_epoch == world_epoch { return Some(message); }
            self.skipped = self.skipped.saturating_add(1);
        }
    }
}
