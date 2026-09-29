//! Voice management: definition resolution, randomization, attenuation, limits and managed loops.

use std::{collections::HashMap, sync::Arc};

use assets::FloatRange;
use bevy::prelude::Resource;

use super::{
    bank::SoundBank,
    settings::{AudioCategory, AudioSettings},
    voice::{VoiceShared, VoiceSource, attenuation, pan_for},
};

/// Concurrent voice ceiling; needs native measurement.
pub(super) const MAX_VOICES: usize = 48;
const MAX_QUEUED: usize = 256;
const MAX_SAME_SOUND: usize = 6;
/// Audible radius of a positional sound without an explicit `max_distance`.
const DEFAULT_MAX_DISTANCE: f32 = 16.0;
/// Seconds a managed loop takes to fade in or out; needs native measurement.
const LOOP_FADE_SECONDS: f32 = 1.5;

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Listener {
    pub position: [f32; 3],
    pub right: [f32; 3],
}

/// A request to play a named sound definition; ranges are sampled once at start.
#[derive(Clone, Debug)]
pub(crate) struct SoundRequest {
    pub name: Arc<str>,
    /// World position in blocks; `None` plays non-positionally.
    pub position: Option<[f32; 3]>,
    pub volume: FloatRange,
    pub pitch: FloatRange,
    pub looping: bool,
}

impl SoundRequest {
    pub(crate) fn new(name: impl Into<Arc<str>>) -> Self {
        Self {
            name: name.into(),
            position: None,
            volume: FloatRange::ONE,
            pitch: FloatRange::ONE,
            looping: false,
        }
    }

    pub(crate) fn at(mut self, position: [f32; 3]) -> Self {
        self.position = Some(position);
        self
    }

    pub(crate) fn with_ranges(mut self, volume: FloatRange, pitch: FloatRange) -> Self {
        self.volume = volume;
        self.pitch = pitch;
        self
    }

    pub(crate) fn scaled(mut self, volume: f32, pitch: f32) -> Self {
        self.volume = FloatRange {
            min: self.volume.min * volume,
            max: self.volume.max * volume,
        };
        self.pitch = FloatRange {
            min: self.pitch.min * pitch,
            max: self.pitch.max * pitch,
        };
        self
    }
}

/// Desired state of one looping ambience channel.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct LoopSpec {
    pub name: Arc<str>,
    pub volume: f32,
}

#[derive(Debug, Default)]
pub(crate) struct EngineStats {
    pub started: u64,
    pub no_bank: u64,
    pub no_definition: u64,
    pub no_pcm: u64,
    pub out_of_range: u64,
    pub no_listener: u64,
    pub voice_limit: u64,
    pub queue_overflow: u64,
    pub stale: u64,
    pub unrouted: u64,
    pub backend_failed: u64,
}

struct Voice {
    shared: Arc<VoiceShared>,
    name: Arc<str>,
    category: AudioCategory,
    /// Definition x alternative x request volume, fixed at start.
    base: f32,
    level: f32,
    target_level: f32,
    fade_per_second: f32,
    position: Option<[f32; 3]>,
    min: f32,
    max: f32,
    key: Option<&'static str>,
    priority: u8,
    gain: f32,
}

fn priority(category: AudioCategory) -> u8 {
    match category {
        AudioCategory::Ui => 4,
        AudioCategory::Music | AudioCategory::Records | AudioCategory::Weather => 3,
        AudioCategory::Ambient => 1,
        _ => 2,
    }
}

#[derive(Resource)]
pub(crate) struct AudioEngine {
    bank: Option<SoundBank>,
    voices: Vec<Voice>,
    queue: Vec<SoundRequest>,
    loops: HashMap<&'static str, LoopSpec>,
    rng: u64,
    pub(crate) stats: EngineStats,
    server_seen: u64,
    clock: f64,
    recent: Vec<(Box<str>, [f32; 3], f64)>,
}

impl Default for AudioEngine {
    fn default() -> Self {
        Self::new(None)
    }
}

impl AudioEngine {
    pub(crate) fn new(bank: Option<SoundBank>) -> Self {
        let seed = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0x9e37_79b9, |elapsed| elapsed.as_nanos() as u64);
        Self {
            bank,
            voices: Vec::new(),
            queue: Vec::new(),
            loops: HashMap::new(),
            rng: seed | 1,
            stats: EngineStats::default(),
            server_seen: 0,
            clock: 0.0,
            recent: Vec::new(),
        }
    }

    /// Records that a sound of class `key` was heard at `position`, for duplicate suppression.
    pub(crate) fn note_recent(&mut self, key: &str, position: [f32; 3]) {
        if self.recent.len() >= 64 {
            self.recent.remove(0);
        }
        self.recent.push((key.into(), position, self.clock));
    }

    /// Whether class `key` was noted within `seconds` and `radius` blocks of `position`.
    pub(crate) fn was_recent(
        &self,
        key: &str,
        position: [f32; 3],
        seconds: f64,
        radius: f32,
    ) -> bool {
        self.recent.iter().any(|(name, at, time)| {
            &**name == key
                && self.clock - time <= seconds
                && (0..3)
                    .map(|axis| (at[axis] - position[axis]).powi(2))
                    .sum::<f32>()
                    <= radius * radius
        })
    }

    pub(crate) fn has_bank(&self) -> bool {
        self.bank.is_some()
    }

    pub(crate) fn bank(&self) -> Option<&SoundBank> {
        self.bank.as_ref()
    }

    /// Uniform sample in `[0, 1)` from an internal SplitMix64 stream.
    pub(crate) fn unit(&mut self) -> f32 {
        self.rng = self.rng.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.rng;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        ((z ^ (z >> 31)) >> 40) as f32 / (1_u64 << 24) as f32
    }

    pub(crate) fn enqueue(&mut self, request: SoundRequest) {
        if self.queue.len() >= MAX_QUEUED {
            self.stats.queue_overflow += 1;
            return;
        }
        self.queue.push(request);
    }

    pub(crate) fn is_playing_category(&self, category: AudioCategory) -> bool {
        self.voices
            .iter()
            .any(|voice| voice.category == category && !voice.shared.finished())
    }

    /// Cancels every voice playing `name`.
    pub(crate) fn stop_named(&mut self, name: &str) {
        for voice in self.voices.iter().filter(|voice| &*voice.name == name) {
            voice.shared.cancel();
        }
    }

    pub(crate) fn stop_all(&mut self) {
        self.queue.clear();
        self.loops.clear();
        for voice in &self.voices {
            voice.shared.cancel();
        }
    }

    /// Declares (or clears) the looping ambience on `key`; reconciled on the next pump.
    pub(crate) fn set_loop(&mut self, key: &'static str, spec: Option<LoopSpec>) {
        match spec {
            Some(spec) => {
                self.loops.insert(key, spec);
            }
            None => {
                self.loops.remove(key);
            }
        }
    }

    pub(crate) fn install_server(&mut self, pack: Option<Arc<super::server::ServerSoundPack>>) {
        if let Some(bank) = self.bank.as_mut() {
            bank.install_server(pack);
        }
    }

    /// Drops the server pack on disconnect unless a newer session already published its own.
    pub(crate) fn clear_server_if_current(&mut self) {
        if super::server::current_generation() == self.server_seen {
            self.install_server(None);
        }
    }

    pub(crate) fn poll_server(&mut self) {
        if let Some(update) = super::server::poll_server_sounds(&mut self.server_seen) {
            self.install_server(update);
        }
    }

    /// Advances fades, starts queued and loop voices, and refreshes every voice's gain and pan.
    pub(crate) fn pump(
        &mut self,
        listener: Option<Listener>,
        dt: f32,
        settings: &AudioSettings,
    ) -> Vec<VoiceSource> {
        self.voices.retain(|voice| !voice.shared.finished());
        self.clock += f64::from(dt.max(0.0));
        let clock = self.clock;
        self.recent.retain(|(_, _, time)| clock - time < 5.0);
        let mut started = Vec::new();
        self.reconcile_loops(&mut started, listener, settings);
        for request in std::mem::take(&mut self.queue) {
            if let Some(source) = self.start(&request, listener, settings, None) {
                started.push(source);
            }
        }
        self.refresh(listener, dt, settings);
        started
    }

    fn reconcile_loops(
        &mut self,
        started: &mut Vec<VoiceSource>,
        listener: Option<Listener>,
        settings: &AudioSettings,
    ) {
        let fade = 1.0 / LOOP_FADE_SECONDS;
        for voice in &mut self.voices {
            let Some(key) = voice.key else { continue };
            match self.loops.get(key) {
                Some(spec) if spec.name == voice.name => {
                    voice.target_level = spec.volume;
                    voice.fade_per_second = fade;
                }
                _ => {
                    voice.target_level = 0.0;
                    voice.fade_per_second = fade;
                    voice.key = None;
                }
            }
        }
        let missing: Vec<(&'static str, LoopSpec)> = self
            .loops
            .iter()
            .filter(|(key, _)| !self.voices.iter().any(|voice| voice.key == Some(**key)))
            .map(|(key, spec)| (*key, spec.clone()))
            .collect();
        for (key, spec) in missing {
            let mut request = SoundRequest::new(Arc::clone(&spec.name));
            request.looping = true;
            if let Some(source) = self.start(&request, listener, settings, Some((key, spec.volume)))
            {
                started.push(source);
            }
        }
    }

    fn refresh(&mut self, listener: Option<Listener>, dt: f32, settings: &AudioSettings) {
        for voice in &mut self.voices {
            let step = voice.fade_per_second * dt;
            voice.level = if step.is_finite() && voice.fade_per_second > 0.0 {
                if voice.level < voice.target_level {
                    (voice.level + step).min(voice.target_level)
                } else {
                    (voice.level - step).max(voice.target_level)
                }
            } else {
                voice.target_level
            };
            let (spatial, pan) = match (voice.position, listener) {
                (Some(position), Some(listener)) => {
                    let delta = [
                        position[0] - listener.position[0],
                        position[1] - listener.position[1],
                        position[2] - listener.position[2],
                    ];
                    let distance =
                        (delta[0] * delta[0] + delta[1] * delta[1] + delta[2] * delta[2]).sqrt();
                    (
                        attenuation(distance, voice.min, voice.max),
                        pan_for(delta, listener.right),
                    )
                }
                (Some(_), None) => (0.0, 0.0),
                (None, _) => (1.0, 0.0),
            };
            voice.gain = settings.effective(voice.category) * voice.base * voice.level * spatial;
            voice.shared.set(voice.gain, pan);
            if voice.key.is_none() && voice.target_level == 0.0 && voice.level == 0.0 {
                voice.shared.cancel();
            }
        }
    }

    fn start(
        &mut self,
        request: &SoundRequest,
        listener: Option<Listener>,
        settings: &AudioSettings,
        managed: Option<(&'static str, f32)>,
    ) -> Option<VoiceSource> {
        if self.bank.is_none() {
            self.stats.no_bank += 1;
            return None;
        }
        let roll = [self.unit(), self.unit(), self.unit()];
        let bank = self.bank.as_mut()?;
        let Some(definition) = bank.definition(&request.name) else {
            self.stats.no_definition += 1;
            return None;
        };
        let alternatives: Vec<&assets::AudioAlternative> = definition
            .alternatives
            .iter()
            .filter(|alt| !(settings.low_memory && alt.load_on_low_memory == Some(false)))
            .collect();
        let total: u32 = alternatives.iter().map(|alt| u32::from(alt.weight)).sum();
        if total == 0 {
            self.stats.no_definition += 1;
            return None;
        }
        let mut pick = (roll[0] * total as f32) as u32;
        let chosen = alternatives
            .iter()
            .find(|alt| {
                let weight = u32::from(alt.weight);
                if pick < weight {
                    true
                } else {
                    pick -= weight;
                    false
                }
            })
            .copied()
            .unwrap_or(alternatives[0]);
        let category = AudioCategory::from_definition(definition.category.as_deref());
        let request_volume = request.volume.sample(roll[1]);
        let volume =
            definition.volume.unwrap_or(1.0) * chosen.volume.unwrap_or(1.0) * request_volume;
        let pitch = (definition.pitch.unwrap_or(1.0)
            * chosen.pitch.unwrap_or(1.0)
            * request.pitch.sample(roll[2]))
        .clamp(0.05, 8.0);
        let flat = matches!(category, AudioCategory::Music | AudioCategory::Ui);
        let positional = request.position.is_some() && chosen.is_3d != Some(false) && !flat;
        let min = definition.min_distance.unwrap_or(0.0).max(0.0);
        let max = definition.max_distance.unwrap_or(DEFAULT_MAX_DISTANCE) * request_volume.max(1.0);
        let stream = chosen.stream == Some(true);
        let path = chosen.name.clone();
        let identifier = definition.identifier.clone();
        let position = positional.then_some(request.position).flatten();
        let mut distance_gain = 1.0;
        if let Some(position) = position {
            let Some(listener) = listener else {
                self.stats.no_listener += 1;
                return None;
            };
            let delta: f32 = (0..3)
                .map(|axis| (position[axis] - listener.position[axis]).powi(2))
                .sum::<f32>()
                .sqrt();
            distance_gain = attenuation(delta, min, max);
            if distance_gain <= 0.0 {
                self.stats.out_of_range += 1;
                return None;
            }
        }
        let same = self
            .voices
            .iter()
            .filter(|voice| *voice.name == *identifier)
            .count();
        if same >= MAX_SAME_SOUND && managed.is_none() {
            self.stats.voice_limit += 1;
            return None;
        }
        let new_priority = priority(category);
        let new_gain = settings.effective(category) * volume * distance_gain;
        if self.voices.len() >= MAX_VOICES {
            let victim = self
                .voices
                .iter()
                .enumerate()
                .min_by(|(_, a), (_, b)| {
                    (a.priority, a.gain)
                        .partial_cmp(&(b.priority, b.gain))
                        .unwrap_or(std::cmp::Ordering::Equal)
                })
                .map(|(index, voice)| (index, voice.priority, voice.gain));
            match victim {
                Some((index, victim_priority, victim_gain))
                    if (new_priority, new_gain) > (victim_priority, victim_gain) =>
                {
                    self.voices[index].shared.cancel();
                    self.voices.swap_remove(index);
                }
                _ => {
                    self.stats.voice_limit += 1;
                    return None;
                }
            }
        }
        let Some(pcm) = self.bank.as_mut()?.pcm(&path, stream) else {
            self.stats.no_pcm += 1;
            return None;
        };
        let (level, target_level, key) = match managed {
            Some((key, target)) => (0.0, target, Some(key)),
            None => (1.0, 1.0, None),
        };
        let shared = VoiceShared::new(if managed.is_some() { 0.0 } else { new_gain }, 0.0, pitch);
        let source = VoiceSource::new(Arc::clone(&pcm), Arc::clone(&shared), request.looping);
        self.voices.push(Voice {
            shared,
            name: Arc::from(&*identifier),
            category,
            base: volume,
            level,
            target_level,
            fade_per_second: if managed.is_some() {
                1.0 / LOOP_FADE_SECONDS
            } else {
                0.0
            },
            position,
            min,
            max,
            key,
            priority: new_priority,
            gain: new_gain,
        });
        self.stats.started += 1;
        Some(source)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::voice::Pcm;
    use assets::{
        AudioAlternative, AudioDefinition, RuntimeAudioCatalog, SoundBankIndex, SoundEventTables,
    };

    fn definition(name: &str, category: &str) -> AudioDefinition {
        AudioDefinition {
            identifier: name.into(),
            category: Some(category.into()),
            subtitle: None,
            min_distance: None,
            max_distance: None,
            volume: None,
            pitch: None,
            use_legacy_max_distance: None,
            alternatives: Box::new([AudioAlternative {
                object_form: false,
                name: format!("sounds/{name}").into(),
                weight: 1,
                volume: None,
                pitch: None,
                is_3d: None,
                stream: None,
                load_on_low_memory: None,
            }]),
        }
    }

    fn engine(names: &[(&str, &str)]) -> AudioEngine {
        let defs: Vec<_> = names
            .iter()
            .map(|(name, category)| definition(name, category))
            .collect();
        let catalog = RuntimeAudioCatalog::decode(
            &assets::encode_audio_catalog([0; 32], [0; 32], &defs).expect("catalog"),
        )
        .expect("decode");
        let bytes = assets::encode_sound_bank(b"{}", b"{}", b"{}", &[]).expect("bank");
        let prefix = assets::sound_bank_prefix_len(&bytes).expect("prefix");
        let index = SoundBankIndex::decode_prefix(&bytes[..prefix]).expect("index");
        let mut bank =
            SoundBank::from_parts(index, SoundEventTables::default(), Some(Arc::new(catalog)));
        for (name, _) in names {
            bank.insert_test_pcm(
                &format!("sounds/{name}"),
                Arc::new(Pcm {
                    channels: 1,
                    rate: 48_000,
                    samples: vec![1000; 4800].into(),
                }),
            );
        }
        AudioEngine::new(Some(bank))
    }

    const LISTENER: Listener = Listener {
        position: [0.0; 3],
        right: [1.0, 0.0, 0.0],
    };

    #[test]
    fn positional_sounds_attenuate_and_drop_beyond_max_distance() {
        let mut engine = engine(&[("dig.stone", "block")]);
        let settings = AudioSettings::default();
        engine.enqueue(SoundRequest::new("dig.stone").at([8.0, 0.0, 0.0]));
        engine.enqueue(SoundRequest::new("dig.stone").at([40.0, 0.0, 0.0]));
        let started = engine.pump(Some(LISTENER), 0.05, &settings);
        assert_eq!(started.len(), 1);
        assert_eq!(engine.stats.out_of_range, 1);
        assert!((engine.voices[0].gain - 0.5).abs() < 1e-3);
    }

    #[test]
    fn interface_sounds_ignore_position_and_listener() {
        let mut engine = engine(&[("random.click", "ui")]);
        engine.enqueue(SoundRequest::new("random.click").at([500.0, 0.0, 0.0]));
        let started = engine.pump(None, 0.05, &AudioSettings::default());
        assert_eq!(started.len(), 1);
        assert_eq!(engine.voices[0].gain, 1.0);
    }

    #[test]
    fn category_sliders_scale_gain_and_unknown_definitions_are_counted() {
        let mut engine = engine(&[("dig.stone", "block")]);
        let mut settings = AudioSettings::default();
        settings.set(AudioCategory::Blocks, 0.25);
        engine.enqueue(SoundRequest::new("dig.stone").at([0.0; 3]));
        engine.enqueue(SoundRequest::new("missing.sound"));
        engine.pump(Some(LISTENER), 0.05, &settings);
        assert!((engine.voices[0].gain - 0.25).abs() < 1e-6);
        assert_eq!(engine.stats.no_definition, 1);
    }

    #[test]
    fn identical_sounds_are_capped() {
        let mut engine = engine(&[("dig.stone", "block")]);
        for _ in 0..10 {
            engine.enqueue(SoundRequest::new("dig.stone").at([1.0, 0.0, 0.0]));
        }
        assert_eq!(
            engine
                .pump(Some(LISTENER), 0.05, &AudioSettings::default())
                .len(),
            MAX_SAME_SOUND
        );
        assert_eq!(engine.stats.voice_limit, 4);
    }

    #[test]
    fn managed_loops_fade_in_and_retire_after_fading_out() {
        let mut engine = engine(&[("ambient.loop", "ambient")]);
        let settings = AudioSettings::default();
        let spec = LoopSpec {
            name: "ambient.loop".into(),
            volume: 0.5,
        };
        engine.set_loop("underwater", Some(spec));
        let mut held = engine.pump(None, 0.5, &settings);
        assert_eq!(held.len(), 1);
        assert!(engine.voices[0].gain > 0.0 && engine.voices[0].gain < 0.5);
        held.extend(engine.pump(None, 5.0, &settings));
        assert_eq!(held.len(), 1);
        assert!((engine.voices[0].gain - 0.5).abs() < 1e-6);
        engine.set_loop("underwater", None);
        engine.pump(None, 5.0, &settings);
        assert!(held[0].next().is_none());
    }

    #[test]
    fn recent_sounds_expire_and_respect_radius() {
        let mut engine = engine(&[("dig.stone", "block")]);
        engine.note_recent("break", [0.0; 3]);
        assert!(engine.was_recent("break", [1.0, 0.0, 0.0], 1.0, 2.0));
        assert!(!engine.was_recent("break", [9.0, 0.0, 0.0], 1.0, 2.0));
        assert!(!engine.was_recent("hurt", [0.0; 3], 1.0, 2.0));
        engine.pump(None, 2.0, &AudioSettings::default());
        assert!(!engine.was_recent("break", [0.0; 3], 1.0, 2.0));
    }

    #[test]
    fn stop_named_cancels_matching_voices() {
        let mut engine = engine(&[("dig.stone", "block")]);
        engine.enqueue(SoundRequest::new("dig.stone").at([1.0, 0.0, 0.0]));
        let mut sources = engine.pump(Some(LISTENER), 0.05, &AudioSettings::default());
        engine.stop_named("dig.stone");
        assert!(sources[0].next().is_none());
    }
}
