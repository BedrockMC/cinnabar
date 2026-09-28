//! The live particle world: emitter registry, spawn API, fixed-budget ticking.

use std::sync::Arc;

use assets::RuntimeParticleAssets;
use bevy::prelude::Resource;

use super::{
    atlas::{ParticleAtlas, Placement},
    def::TextureSource,
    emitter::{Emitter, Outputs, ParticleSound, SpawnRequest},
    library::EffectLibrary,
    world::ParticleWorld,
};

/// Live particles across all emitters; the oldest are dropped past this.
pub const MAX_LIVE_PARTICLES: usize = 8192;
pub const MAX_EMITTERS: usize = 768;
/// Spawn requests farther than this from the camera are dropped.
pub const MAX_SPAWN_DISTANCE: f32 = 128.0;
/// Longest single simulation step; larger frame gaps are split.
const MAX_STEP: f32 = 1.0 / 30.0;
const MAX_FRAME_SECONDS: f32 = 0.25;

#[derive(Resource, Default)]
pub struct ParticleSystem {
    library: EffectLibrary,
    atlas: ParticleAtlas,
    emitters: Vec<Emitter>,
    sounds: Vec<ParticleSound>,
    next_id: u64,
    seed: u64,
    camera: [f32; 3],
    boxes: Vec<[f32; 6]>,
    /// Requests dropped for an unknown effect, distance or a full emitter table.
    pub dropped_spawns: u64,
    pub daylight: f32,
}

impl ParticleSystem {
    /// Builds the system from the compiled carrier; effects parse eagerly.
    #[must_use]
    pub fn from_assets(assets: &RuntimeParticleAssets) -> Self {
        Self {
            library: EffectLibrary::from_assets(assets),
            atlas: ParticleAtlas::from_assets(assets),
            daylight: 1.0,
            ..Self::default()
        }
    }

    /// Registers or overrides an effect from raw json (joined server packs).
    pub fn register_effect(&mut self, bytes: &[u8]) -> bool {
        self.library.insert(bytes)
    }

    #[must_use]
    pub fn has_effect(&self, name: &str) -> bool {
        self.resolve(name).is_some()
    }

    #[must_use]
    pub fn effect_count(&self) -> usize {
        self.library.len()
    }

    #[must_use]
    pub fn atlas(&self) -> &ParticleAtlas {
        &self.atlas
    }

    pub fn set_camera(&mut self, position: [f32; 3]) {
        self.camera = position;
    }

    #[must_use]
    pub fn camera(&self) -> [f32; 3] {
        self.camera
    }

    #[must_use]
    pub fn live_particles(&self) -> usize {
        self.emitters.iter().map(|e| e.particles.len()).sum()
    }

    #[must_use]
    pub fn emitter_count(&self) -> usize {
        self.emitters.len()
    }

    /// Removes every emitter and particle (dimension change, disconnect).
    pub fn clear(&mut self) {
        self.emitters.clear();
        self.sounds.clear();
    }

    pub(super) fn emitters_mut(&mut self) -> &mut [Emitter] {
        &mut self.emitters
    }

    fn resolve(&self, name: &str) -> Option<&Arc<super::def::EffectDef>> {
        self.library.get(name).or_else(|| {
            if name.contains(':') {
                None
            } else {
                self.library.get(&format!("minecraft:{name}"))
            }
        })
    }

    /// Starts an emitter; returns its id, or `None` when the effect is unknown, too far away,
    /// or the emitter table is full.
    pub fn spawn(&mut self, request: &SpawnRequest) -> Option<u64> {
        let Some(def) = self.resolve(&request.effect).cloned() else {
            self.dropped_spawns += 1;
            return None;
        };
        let distance_sq: f32 = (0..3)
            .map(|i| (request.position[i] - self.camera[i]).powi(2))
            .sum();
        if !distance_sq.is_finite() || distance_sq > MAX_SPAWN_DISTANCE * MAX_SPAWN_DISTANCE {
            self.dropped_spawns += 1;
            return None;
        }
        if self.emitters.len() >= MAX_EMITTERS {
            self.emitters.remove(0);
        }
        let mut request = request.clone();
        let texture = self.resolve_texture(&def.texture, &mut request);
        self.next_id += 1;
        let id = self.next_id;
        let seed = request.seed
            ^ self.seed.wrapping_mul(0x9E37_79B9_7F4A_7C15)
            ^ id.wrapping_mul(0xD6E8_FEB8_6659_FD93)
            ^ u64::from(request.position[0].to_bits())
            ^ (u64::from(request.position[2].to_bits()) << 32);
        self.emitters
            .push(Emitter::new(id, def, texture, &request, seed));
        Some(id)
    }

    fn resolve_texture(&mut self, source: &TextureSource, request: &mut SpawnRequest) -> Placement {
        let fallback = self.atlas.fallback();
        match source {
            TextureSource::Path(path) => self.atlas.placement(path).unwrap_or(fallback),
            TextureSource::Terrain | TextureSource::Items => {
                let Some(tile) = request.tile.as_ref() else {
                    return fallback;
                };
                let Some(placement) = self.atlas.tile(tile.key, tile.size, &tile.pixels) else {
                    return fallback;
                };
                let [u, v, du, dv] = placement.normalized();
                for (name, value) in [
                    ("emitter_texture_coordinate.u", u),
                    ("emitter_texture_coordinate.v", v),
                    ("emitter_texture_size.u", du),
                    ("emitter_texture_size.v", dv),
                ] {
                    request.variables.push((name.to_owned(), value));
                }
                placement
            }
        }
    }

    /// Moves an attached emitter; ignored once the emitter is gone.
    pub fn set_transform(&mut self, id: u64, position: [f32; 3], basis: [[f32; 3]; 3]) {
        if let Some(emitter) = self.emitters.iter_mut().find(|e| e.id == id) {
            emitter.pos = position;
            emitter.basis = basis;
        }
    }

    /// Stops an emitter from emitting; its live particles finish out.
    pub fn stop(&mut self, id: u64) {
        if let Some(emitter) = self.emitters.iter_mut().find(|e| e.id == id) {
            emitter.done = true;
        }
    }

    /// Sound requests raised by particle events since the last call.
    pub fn take_sounds(&mut self) -> Vec<ParticleSound> {
        std::mem::take(&mut self.sounds)
    }

    /// Advances every emitter by `dt` seconds against `world`.
    pub fn tick(&mut self, dt: f32, world: &dyn ParticleWorld) {
        if !dt.is_finite() || dt <= 0.0 {
            return;
        }
        let mut remaining = dt.min(MAX_FRAME_SECONDS);
        while remaining > 1e-6 {
            let step = remaining.min(MAX_STEP);
            remaining -= step;
            self.step(step, world);
        }
        self.enforce_budget();
    }

    fn step(&mut self, dt: f32, world: &dyn ParticleWorld) {
        let mut output = Outputs::default();
        let live = self.live_particles();
        let mut budget = MAX_LIVE_PARTICLES.saturating_sub(live);
        let mut boxes = std::mem::take(&mut self.boxes);
        for emitter in &mut self.emitters {
            let before = emitter.particles.len();
            emitter.advance(dt, &mut output, budget);
            budget = budget.saturating_sub(emitter.particles.len().saturating_sub(before));
            emitter.update_particles(dt, world, &mut boxes, &mut output);
        }
        self.boxes = boxes;
        self.emitters.retain(|emitter| !emitter.is_finished());
        self.sounds.append(&mut output.sounds);
        self.seed = self.seed.wrapping_add(1);
        for request in output.spawns {
            self.spawn(&request);
        }
    }

    /// Drops the oldest particles until the live count fits the budget.
    fn enforce_budget(&mut self) {
        let mut excess = self.live_particles().saturating_sub(MAX_LIVE_PARTICLES);
        while excess > 0 {
            let Some(victim) = self
                .emitters
                .iter_mut()
                .filter(|e| !e.particles.is_empty())
                .min_by_key(|e| e.id)
            else {
                return;
            };
            let take = excess.min(victim.particles.len());
            victim.particles.drain(..take);
            excess -= take;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::particles::world::EmptyWorld;

    const EFFECT: &str = r#"{"particle_effect":{"description":{"identifier":"minecraft:burst","basic_render_parameters":{"material":"particles_alpha","texture":"x"}},
      "components":{
        "minecraft:emitter_lifetime_once":{"active_time":1},
        "minecraft:emitter_rate_instant":{"num_particles":4},
        "minecraft:emitter_shape_point":{},
        "minecraft:particle_lifetime_expression":{"max_lifetime":0.5},
        "minecraft:particle_appearance_billboard":{"size":[0.1,0.1],"facing_camera_mode":"lookat_xyz","uv":{"uv":[0,0],"uv_size":[1,1]}}}}}"#;

    fn system() -> ParticleSystem {
        let mut system = ParticleSystem::default();
        assert!(system.register_effect(EFFECT.as_bytes()));
        system
    }

    fn request(name: &str, x: f32) -> SpawnRequest {
        SpawnRequest {
            effect: name.into(),
            position: [x, 0.0, 0.0],
            ..SpawnRequest::default()
        }
    }

    #[test]
    fn spawn_resolves_bare_names_and_bursts_particles() {
        let mut system = system();
        assert!(system.spawn(&request("burst", 0.0)).is_some());
        system.tick(0.05, &EmptyWorld);
        assert_eq!(system.live_particles(), 4);
    }

    #[test]
    fn unknown_and_distant_effects_are_dropped_and_counted() {
        let mut system = system();
        assert!(system.spawn(&request("missing", 0.0)).is_none());
        assert!(system.spawn(&request("burst", 500.0)).is_none());
        assert_eq!(system.dropped_spawns, 2);
    }

    #[test]
    fn finished_emitters_are_removed_once_particles_die() {
        let mut system = system();
        system.spawn(&request("burst", 0.0));
        system.tick(0.1, &EmptyWorld);
        assert_eq!(system.emitter_count(), 1);
        system.tick(1.0, &EmptyWorld);
        assert_eq!(system.emitter_count(), 0);
    }

    #[test]
    fn oldest_particles_are_dropped_past_the_budget() {
        let mut system = ParticleSystem::default();
        let heavy = EFFECT
            .replace("minecraft:burst", "minecraft:heavy")
            .replace("\"num_particles\":4", "\"num_particles\":100");
        assert!(system.register_effect(heavy.as_bytes()));
        for _ in 0..(MAX_LIVE_PARTICLES / 100 + 20) {
            system.spawn(&request("heavy", 0.0));
        }
        system.tick(0.02, &EmptyWorld);
        assert!(system.live_particles() <= MAX_LIVE_PARTICLES);
    }
}
