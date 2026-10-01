//! Native declarative playback controller, independent of a downloaded component.

use std::{collections::BTreeSet, sync::{Arc, atomic::AtomicU64}};
use anyhow::{Result, ensure};
use crate::{bundle::VerifiedBundle, manifest::Permission, negotiation::Grant, runtime::Principal};
use super::{clock::{Clock, Correction, correction}, descriptor::Descriptor,
    frames::{FrameQueue, PcmBlock, VideoFrame}, timeline::{Message, Playback}};

pub struct Player {
    owner: Principal,
    epoch: u64,
    expires_unix: u64,
    descriptor: Descriptor,
    origins: BTreeSet<String>,
    data_budget: Arc<AtomicU64>,
    clock: Clock,
    ping: Option<(u64, u64)>,
    ping_id: u64,
    last_ping_us: u64,
    playback: Playback,
    frames: FrameQueue,
    pcm: Option<PcmBlock>,
    #[cfg(feature = "developer-media")]
    worker: Option<super::webm::Worker>,
    decoder_generation: u64,
    pub buffering: bool,
}

impl Player {
    /// Resolves a media ID only from a verified bundle and an unexpired, consented grant.
    pub fn prepare(bundle: &VerifiedBundle, path: &str, grant: &Grant, epoch: u64, now_unix: u64, data_budget: Arc<AtomicU64>) -> Result<Self> {
        ensure!(now_unix < grant.expires_unix && grant.offer.offer.scope.permissions.contains(&Permission::Media)
            && bundle.manifest.permissions.contains(&Permission::Media), "media permission denied");
        ensure!(grant.offer.offer.packages.iter().any(|p| p.id == bundle.manifest.id
            && p.publisher_key == bundle.manifest.publisher_key), "foreign bundle");
        let bytes = bundle.file(path).ok_or_else(|| anyhow::anyhow!("undeclared media descriptor"))?;
        ensure!(bytes.len() <= crate::policy::MAX_MARKER_BYTES, "media descriptor too large");
        let descriptor: Descriptor = serde_json::from_slice(bytes)?;
        descriptor.validate(&grant.offer.offer.scope.origins)?;
        ensure!(bundle.file(&descriptor.poster).is_some(), "fallback poster missing");
        Ok(Self {
            owner: Principal { session: grant.session.clone(), bundle: bundle.manifest.id.clone(), generation: 1 },
            epoch, expires_unix: grant.expires_unix, descriptor,
            origins: grant.offer.offer.scope.origins.clone(), data_budget,
            clock: Clock::default(), ping: None, ping_id: 0, last_ping_us: 0,
            playback: Playback::default(), frames: FrameQueue::default(), pcm: None,
            #[cfg(feature = "developer-media")]
            worker: None,
            decoder_generation: 0, buffering: true,
        })
    }

    /// Issues coarse clock probes only on an already negotiated extension channel.
    pub fn ping(&mut self, now_us: u64) -> Option<(u64, u64)> {
        if now_us.saturating_sub(self.last_ping_us) < 1_000_000 { return None; }
        self.ping_id = self.ping_id.checked_add(1)?;
        self.last_ping_us = now_us;
        self.ping = Some((self.ping_id, now_us));
        self.ping
    }

    /// Accepts only a response to this player's outstanding probe.
    pub fn clock_reply(&mut self, id: u64, c0: u64, s1: u64, s2: u64, c3: u64) -> Result<()> {
        ensure!(self.ping.take() == Some((id, c0)), "unsolicited clock reply");
        self.clock.observe(c0, s1, s2, c3)
    }

    /// Queues a validated server control without starting network or decode work.
    pub fn control(&mut self, message: Message, local_us: u64) -> Result<()> {
        let (server_us, _) = self.clock.server_now(local_us).ok_or_else(|| anyhow::anyhow!("media clock unavailable"))?;
        if let super::timeline::Operation::Prepare { media_id } = &message.operation {
            ensure!(media_id == &self.descriptor.id, "unknown media ID");
        }
        self.playback.enqueue(message, &self.owner, self.epoch, &self.descriptor.timeline, server_us)
    }

    /// Services buffered output without waiting; production decoding remains unavailable.
    pub fn tick(&mut self, now_unix: u64, local_us: u64, autoplay: bool) -> Result<Correction> {
        ensure!(now_unix < self.expires_unix, "media grant expired");
        let Some((server_us, _)) = self.clock.server_now(local_us) else { self.buffering = true; return Ok(Correction::Hold); };
        self.playback.advance(server_us, self.descriptor.duration_us)?;
        let desired = self.playback.position(server_us, self.descriptor.duration_us);
        if !autoplay || self.playback.stopped {
            self.stop_decoder();
            return Ok(Correction::Hold);
        }
        #[cfg(feature = "developer-media")]
        {
            if self.decoder_generation != self.playback.decode_generation {
                self.stop_decoder();
                if !super::webm::Worker::available() { return Ok(Correction::Hold); }
                self.worker = Some(super::webm::Worker::start(self.descriptor.clone(), self.origins.clone(),
                    self.playback.decode_generation, Arc::clone(&self.data_budget), desired)?);
                self.decoder_generation = self.playback.decode_generation;
            }
            if self.pcm.is_none() && self.frames.has_capacity()
                && let Some(output) = self.worker.as_ref().and_then(|worker| worker.poll()) {
                match output? {
                    super::webm::Output::Video(frame) => self.frames.push(frame, self.decoder_generation)?,
                    super::webm::Output::Audio(block) => { block.validate(self.decoder_generation)?; self.pcm = Some(block); }
                    super::webm::Output::End => {}
                }
            }
        }
        #[cfg(not(feature = "developer-media"))]
        {
            let _ = (&self.origins, &self.data_budget, desired);
            anyhow::bail!("WebM decoder is not enabled; retain fallback poster");
        }
        #[cfg(feature = "developer-media")]
        Ok(Correction::Hold)
    }

    /// Presents against audible audio when available, otherwise the shared monotonic timeline.
    pub fn video(&mut self, local_us: u64, audible_us: Option<u64>) -> Option<VideoFrame> {
        let (server_us, _) = self.clock.server_now(local_us)?;
        let desired = self.playback.position(server_us, self.descriptor.duration_us);
        let frame = self.frames.present(audible_us.unwrap_or(desired), self.decoder_generation);
        self.buffering = frame.is_none() && self.buffering;
        frame
    }

    /// Reports a drift decision; an output adapter must perform rate or seek correction.
    pub fn drift(&self, local_us: u64, audible_us: u64) -> Option<Correction> {
        let (server_us, _) = self.clock.server_now(local_us)?;
        Some(correction(audible_us, self.playback.position(server_us, self.descriptor.duration_us)))
    }

    /// Hands one bounded block to the host mixer, never to guest code.
    pub fn take_pcm(&mut self) -> Option<PcmBlock> { self.pcm.take() }

    /// Exposes authoritative pause, gain and surface state to trusted adapters.
    pub fn playback(&self) -> &Playback { &self.playback }

    /// Drops all generation-owned decode output immediately.
    fn stop_decoder(&mut self) {
        #[cfg(feature = "developer-media")]
        { self.worker = None; }
        self.frames = FrameQueue::default();
        self.pcm = None;
        self.buffering = true;
        self.decoder_generation = 0;
    }
}
