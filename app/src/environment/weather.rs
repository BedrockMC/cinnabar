//! Client-side weather presentation: eased levels, lightning flash and the precipitation scene.

use assets::BiomeRule;
use bevy::{
    prelude::{Local, Query, Res, ResMut, Resource, Time, Transform, With},
    time::Real,
};
use client_world::WorldStream;
use meshing::CameraMedium;
use render::{
    AtmosphereFrame, ColumnSample, ColumnSampler, LightningScene, OcclusionGrid,
    PRECIPITATION_LEVEL_PER_SECOND, PRECIPITATION_SAMPLE_OFFSETS, PRECIPITATION_TICKS_PER_SECOND,
    PrecipitationMix, PrecipitationScene, PrecipitationSim, RainSplashQueue, SkyKind,
    approach_level, average_precipitation, lightning_bolt_segments, lightning_flash_level,
    pick_rain_splashes, precipitation_forward_offset, push_bolt_records,
};

use super::WeatherState;
use crate::{camera::FlyCamera, runtime::world::ClientWorld};

const MAX_FRAME_STEP_SECONDS: f64 = 1.0;
const MAX_QUEUED_SPLASHES: usize = 256;
/// Ticks a bolt stays drawn after it spawns; needs native measurement.
const BOLT_VISIBLE_TICKS: u32 = 8;

const WEATHER_TEXTURES_FILENAME: &str = "vanilla-v1.mcbewth";
const WEATHER_TEXTURES_COMPILE_COMMAND: &str = "make weather-assets";

/// Loads the optional precipitation and End sky carrier next to the world carrier; an absent or
/// invalid carrier logs a notice and leaves the procedural fallbacks in place.
#[must_use]
pub(crate) fn load_optional_weather_textures(
    world_asset_path: &std::path::Path,
) -> render::WeatherTextureAssets {
    let path = world_asset_path.with_file_name(WEATHER_TEXTURES_FILENAME);
    let decoded = std::fs::read(&path)
        .map_err(|error| error.to_string())
        .and_then(|bytes| {
            assets::decode_weather_textures(&bytes).map_err(|error| error.to_string())
        });
    match decoded {
        Ok((textures, identity)) => {
            eprintln!("loaded weather textures from {}", path.display());
            render::WeatherTextureAssets::new(std::sync::Arc::new(textures), identity)
        }
        Err(error) => {
            eprintln!(
                "weather textures unavailable at {} ({error}); using procedural precipitation and End sky; build with {WEATHER_TEXTURES_COMPILE_COMMAND}",
                path.display()
            );
            render::WeatherTextureAssets::default()
        }
    }
}

/// Time of the latest lightning strike; the sky and lightmap flash for a moment after it.
#[derive(Resource, Debug, Default)]
pub(crate) struct LightningFlashState {
    struck_at: Option<f64>,
}

impl LightningFlashState {
    /// Starts a flash; called when a lightning bolt actor appears.
    pub(crate) fn trigger(&mut self, elapsed_seconds: f64) {
        self.struck_at = Some(elapsed_seconds);
    }

    pub(crate) fn level(&self, elapsed_seconds: f64) -> f32 {
        self.struck_at.map_or(0.0, |struck| {
            lightning_flash_level((elapsed_seconds - struck) as f32)
        })
    }
}

/// Rain and thunder levels eased toward the server targets, plus time spent submerged.
#[derive(Debug, Default)]
pub(crate) struct WeatherDisplay {
    generation: Option<u64>,
    rain: f32,
    thunder: f32,
    last_elapsed: Option<f64>,
    step_seconds: f64,
    submerged: f32,
}

impl WeatherDisplay {
    /// Moves the displayed levels toward `target`; a new session snaps to it.
    pub(crate) fn advance(&mut self, target: WeatherState, elapsed_seconds: f64) -> WeatherState {
        let previous = self.last_elapsed.replace(elapsed_seconds);
        self.step_seconds = previous.map_or(0.0, |previous| {
            (elapsed_seconds - previous).clamp(0.0, MAX_FRAME_STEP_SECONDS)
        });
        if self.generation != Some(target.session_generation) {
            self.generation = Some(target.session_generation);
            self.rain = target.rain_level;
            self.thunder = target.lightning_level;
        } else {
            let step = PRECIPITATION_LEVEL_PER_SECOND * self.step_seconds as f32;
            self.rain = approach_level(self.rain, target.rain_level, step);
            self.thunder = approach_level(self.thunder, target.lightning_level, step);
        }
        WeatherState {
            rain_level: self.rain,
            lightning_level: self.thunder,
            ..target
        }
    }

    /// Seconds continuously spent in water as of the last `advance`; zero elsewhere.
    pub(crate) fn submerged_seconds(&mut self, medium: CameraMedium) -> f32 {
        if medium == CameraMedium::Water {
            self.submerged += self.step_seconds as f32;
        } else {
            self.submerged = 0.0;
        }
        self.submerged
    }
}

struct StreamColumns<'a> {
    stream: &'a WorldStream,
    rules: &'a [BiomeRule],
}

impl ColumnSampler for StreamColumns<'_> {
    fn sample(&mut self, x: i32, z: i32) -> Option<ColumnSample> {
        let top = self.stream.top_non_air_block_y(x, z)?;
        let biome =
            self.stream
                .camera_biome_id([x as f32 + 0.5, top as f32 + 0.5, z as f32 + 0.5])?;
        let rule = &self.rules[self
            .rules
            .binary_search_by_key(&biome, |rule| rule.id)
            .ok()?];
        Some(ColumnSample {
            surface_y: top.saturating_add(1),
            temperature: rule.temperature(),
            downfall: rule.downfall(),
        })
    }
}

/// Occlusion columns re-sampled per tick; the whole grid refreshes about every 0.8 s.
const OCCLUSION_REFRESH_PER_TICK: usize = 256;
/// Most simulation ticks replayed after a stall.
const MAX_CATCH_UP_TICKS: u64 = 10;
const PRECIPITATION_SEED: u64 = 0x5241_494e;

pub(crate) struct PrecipitationCadence {
    sim: PrecipitationSim,
    grid: OcclusionGrid,
    cursor: usize,
    last_tick: Option<u64>,
}

impl Default for PrecipitationCadence {
    fn default() -> Self {
        Self {
            sim: PrecipitationSim::new(PRECIPITATION_SEED),
            grid: OcclusionGrid::default(),
            cursor: 0,
            last_tick: None,
        }
    }
}

/// Biome (temperature, downfall) at a block position, when its chunk and biome are known.
fn biome_climate(
    stream: &WorldStream,
    rules: &[BiomeRule],
    position: [f32; 3],
) -> Option<(f32, f32)> {
    let id = stream.camera_biome_id(position)?;
    let rule = &rules[rules.binary_search_by_key(&id, |rule| rule.id).ok()?];
    Some((rule.temperature(), rule.downfall()))
}

/// Ticks the vanilla precipitation layers, refreshes the occlusion grid and queues rain splashes.
#[allow(clippy::too_many_arguments)]
pub(crate) fn update_precipitation_scene(
    frame: Res<AtmosphereFrame>,
    client_world: Res<ClientWorld>,
    camera: Query<&Transform, With<FlyCamera>>,
    time: Res<Time<Real>>,
    mut scene: ResMut<PrecipitationScene>,
    mut splashes: ResMut<RainSplashQueue>,
    mut mix: ResMut<PrecipitationMix>,
    mut cadence: Local<PrecipitationCadence>,
) {
    let level = frame.rain_level();
    let stream = client_world.stream.as_ref();
    let camera = camera.single().ok();
    let (Some(stream), Some(camera), true) = (
        stream,
        camera,
        level > 0.0 && frame.sky_kind() == SkyKind::Overworld,
    ) else {
        scene.layers.clear();
        *mix = PrecipitationMix::default();
        *cadence = PrecipitationCadence::default();
        return;
    };
    let elapsed = time.elapsed_secs_f64();
    let ticks = elapsed * PRECIPITATION_TICKS_PER_SECOND;
    let tick = ticks as u64;
    let origin = camera.translation.to_array();
    let rules = &client_world.runtime_assets.biome_assets().rules;
    let pending = cadence
        .last_tick
        .map_or(1, |last| tick.saturating_sub(last).min(MAX_CATCH_UP_TICKS));
    if pending > 0 {
        cadence.last_tick = Some(tick);
        let feet = origin.map(f32::floor);
        let samples = PRECIPITATION_SAMPLE_OFFSETS.map(|offset| {
            let position = [
                feet[0] + offset[0] as f32,
                feet[1] + offset[1] as f32,
                feet[2] + offset[2] as f32,
            ];
            biome_climate(stream, rules, position)
                .map(|(temperature, downfall)| (temperature, downfall, position[1] as i32))
        });
        let averaged = average_precipitation(&samples);
        *mix = PrecipitationMix {
            rain: averaged.rain * level,
            snow: averaged.snow * level,
        };
        let weights = averaged.lattice_weights(level);
        for step in 0..pending {
            let seconds = (tick - (pending - 1 - step)) as f64 / PRECIPITATION_TICKS_PER_SECOND;
            cadence.sim.tick(weights, seconds as f32);
        }
        let PrecipitationCadence { grid, cursor, .. } = &mut *cadence;
        let mut columns = StreamColumns { stream, rules };
        let refresh = OCCLUSION_REFRESH_PER_TICK * pending as usize;
        grid.update(
            OcclusionGrid::origin_for(origin),
            &mut columns,
            cursor,
            refresh,
        );
        scene.occlusion = std::sync::Arc::new(grid.clone());
        scene.occlusion_generation = scene.occlusion_generation.wrapping_add(1);
        if splashes.positions.len() > MAX_QUEUED_SPLASHES {
            splashes.positions.clear();
        }
        let mut picked = Vec::new();
        pick_rain_splashes(grid, level, tick, &mut picked);
        splashes.positions.extend(picked);
    }
    scene.forward_offset = precipitation_forward_offset(camera.forward().as_vec3().to_array());
    let PrecipitationScene {
        layers,
        forward_offset,
        ..
    } = &mut *scene;
    cadence.sim.frame(
        camera.translation.as_dvec3().to_array(),
        *forward_offset,
        ticks.fract() as f32,
        layers,
    );
}

/// Flashes the sky when a lightning-bolt actor first appears and draws live bolts.
pub(crate) fn update_lightning(
    client_world: Res<ClientWorld>,
    time: Res<Time<Real>>,
    mut flash: ResMut<LightningFlashState>,
    mut scene: ResMut<LightningScene>,
    mut seen: Local<std::collections::HashSet<i64>>,
) {
    scene.records.clear();
    let Some(stream) = client_world.stream.as_ref() else {
        seen.clear();
        return;
    };
    let bolts = stream.lightning_bolts();
    seen.retain(|id| bolts.iter().any(|bolt| bolt.unique_id == *id));
    for bolt in bolts {
        if seen.insert(bolt.unique_id) {
            flash.trigger(time.elapsed_secs_f64());
        }
        if bolt.age_ticks >= BOLT_VISIBLE_TICKS {
            continue;
        }
        let intensity = if bolt.age_ticks % 2 == 0 { 1.0 } else { 0.6 };
        push_bolt_records(
            &lightning_bolt_segments(bolt.unique_id as u64, bolt.position),
            intensity,
            &mut scene.records,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::environment::{WorldClock, replace_session};
    use protocol::WorldEnvironmentBootstrap;

    fn target(rain: f32, thunder: f32, generation: u64) -> WeatherState {
        let mut weather = WeatherState::default();
        let mut clock = WorldClock::default();
        replace_session(
            &mut clock,
            &mut weather,
            WorldEnvironmentBootstrap {
                initial_time: 0,
                day_cycle_lock_time: 0,
                daylight_cycle_enabled: true,
                rain_level: rain,
                lightning_level: thunder,
            },
            0.0,
        );
        weather.session_generation = generation;
        weather
    }

    #[test]
    fn first_frame_snaps_then_levels_ease_at_a_fixed_rate() {
        let mut display = WeatherDisplay::default();
        assert_eq!(display.advance(target(1.0, 0.0, 1), 10.0).rain_level(), 1.0);
        let fading = display.advance(target(0.0, 0.0, 1), 11.0);
        assert!((fading.rain_level() - 0.8).abs() < 1.0e-6);
        let mut done = fading;
        for second in 12..20 {
            done = display.advance(target(0.0, 0.0, 1), f64::from(second));
        }
        assert_eq!(done.rain_level(), 0.0);
    }

    #[test]
    fn a_new_session_snaps_instead_of_fading() {
        let mut display = WeatherDisplay::default();
        display.advance(target(1.0, 1.0, 1), 0.0);
        let next = display.advance(target(0.0, 0.0, 2), 0.1);
        assert_eq!((next.rain_level(), next.lightning_level()), (0.0, 0.0));
    }

    #[test]
    fn submerged_time_accumulates_only_in_water() {
        let mut display = WeatherDisplay::default();
        display.advance(target(0.0, 0.0, 1), 0.0);
        display.advance(target(0.0, 0.0, 1), 1.0);
        assert_eq!(display.submerged_seconds(CameraMedium::Water), 1.0);
        display.advance(target(0.0, 0.0, 1), 2.0);
        assert_eq!(display.submerged_seconds(CameraMedium::Water), 2.0);
        assert_eq!(display.submerged_seconds(CameraMedium::Air), 0.0);
    }

    #[test]
    fn lightning_flash_starts_bright_and_expires() {
        let mut flash = LightningFlashState::default();
        assert_eq!(flash.level(5.0), 0.0);
        flash.trigger(5.0);
        assert_eq!(flash.level(5.0), 1.0);
        assert_eq!(flash.level(6.0), 0.0);
    }
}
