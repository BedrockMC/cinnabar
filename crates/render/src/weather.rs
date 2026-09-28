//! Precipitation model: biome classification, per-column surface limits, animation clock, splash hook.
//!
//! Thresholds, radii and speeds are provisional observations and need native calibration.

use bevy::{prelude::Resource, render::extract_resource::ExtractResource};

use crate::celestial::unit;

/// Columns sampled on each side of the camera.
pub const PRECIPITATION_RADIUS: i32 = 10;
/// Blocks the falling sheet extends above the camera.
pub const PRECIPITATION_ABOVE_CAMERA: f32 = 12.0;
/// Upper bound on visible columns, sizing the GPU record buffer.
pub const MAX_PRECIPITATION_COLUMNS: usize =
    ((2 * PRECIPITATION_RADIUS + 1) * (2 * PRECIPITATION_RADIUS + 1)) as usize;
/// Rain level change per second while the server target moves.
pub const PRECIPITATION_LEVEL_PER_SECOND: f32 = 0.2;

const SNOW_TEMPERATURE: f32 = 0.15;
const TEMPERATURE_LOSS_PER_BLOCK_ABOVE_SEA: f32 = 0.05 / 30.0;
const SEA_LEVEL: f32 = 64.0;
const CLOCK_WRAP_SECONDS: f64 = 4096.0;
const MAX_SPLASHES_PER_TICK: f32 = 4.0;

/// What falls in one column.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Precipitation {
    None,
    Rain,
    Snow,
}

/// Temperature after the altitude lapse that turns rain into snow on high ground.
#[must_use]
pub fn altitude_adjusted_temperature(temperature: f32, surface_y: i32) -> f32 {
    let above = (surface_y as f32 - SEA_LEVEL).max(0.0);
    temperature - above * TEMPERATURE_LOSS_PER_BLOCK_ABOVE_SEA
}

/// Rain, snow or nothing for a biome: zero downfall never precipitates, cold falls as snow.
#[must_use]
pub fn classify_precipitation(temperature: f32, downfall: f32, surface_y: i32) -> Precipitation {
    if !temperature.is_finite() || !downfall.is_finite() || downfall <= 0.0 {
        return Precipitation::None;
    }
    if altitude_adjusted_temperature(temperature, surface_y) < SNOW_TEMPERATURE {
        Precipitation::Snow
    } else {
        Precipitation::Rain
    }
}

/// Moves `current` toward `target` by at most `max_step`; invalid input reads as clear.
#[must_use]
pub fn approach_level(current: f32, target: f32, max_step: f32) -> f32 {
    let (current, target) = (unit(current), unit(target));
    let step = if max_step.is_finite() {
        max_step.max(0.0)
    } else {
        0.0
    };
    current + (target - current).clamp(-step, step)
}

/// Wrapped animation seconds, so single-precision scroll stays accurate in long sessions.
#[must_use]
pub fn precipitation_clock(elapsed_seconds: f64) -> f32 {
    if elapsed_seconds.is_finite() {
        elapsed_seconds.rem_euclid(CLOCK_WRAP_SECONDS) as f32
    } else {
        0.0
    }
}

/// Horizontal drift per block of fall; a slow, bounded sway.
#[must_use]
pub fn precipitation_wind(clock: f32) -> [f32; 2] {
    [
        0.08 * (clock * 0.11).sin() + 0.04 * (clock * 0.37).sin(),
        0.06 * (clock * 0.13 + 1.7).sin(),
    ]
}

/// One visible column: its surface height and what falls, in the layout the GPU reads.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct PrecipitationColumn {
    pub x: i32,
    pub z: i32,
    /// Height the sheet stops at: the first block above the top non-air block.
    pub bottom_y: f32,
    /// Bit 0 marks snow; bits 8..16 hold the distance-fade alpha.
    pub flags: u32,
}

impl PrecipitationColumn {
    const SNOW_FLAG: u32 = 1;

    #[must_use]
    pub fn new(x: i32, z: i32, bottom_y: f32, kind: Precipitation, alpha: f32) -> Self {
        let snow = u32::from(kind == Precipitation::Snow) * Self::SNOW_FLAG;
        Self {
            x,
            z,
            bottom_y,
            flags: snow | (((unit(alpha) * 255.0).round() as u32) << 8),
        }
    }

    #[must_use]
    pub fn is_snow(&self) -> bool {
        self.flags & Self::SNOW_FLAG != 0
    }

    #[must_use]
    pub fn alpha(&self) -> f32 {
        ((self.flags >> 8) & 0xff) as f32 / 255.0
    }
}

/// Surface facts of one world column, supplied by the world-stream owner.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ColumnSample {
    /// Y of the first block above the top non-air block.
    pub surface_y: i32,
    pub temperature: f32,
    pub downfall: f32,
}

/// Lookup of loaded world columns; `None` for unloaded ones.
pub trait ColumnSampler {
    fn sample(&mut self, x: i32, z: i32) -> Option<ColumnSample>;
}

/// Precipitation state consumed by the render world.
#[derive(Resource, ExtractResource, Clone, Debug, Default, PartialEq)]
pub struct PrecipitationScene {
    pub columns: Vec<PrecipitationColumn>,
    /// Smoothed rain level in `0..=1` scaling the sheet opacity.
    pub level: f32,
    /// Wrapped animation seconds.
    pub clock: f32,
}

/// Fills `out` with the columns of the disc around `camera` that precipitate and sit below the sheet top.
pub fn build_precipitation_columns(
    sampler: &mut impl ColumnSampler,
    camera: [f32; 3],
    out: &mut Vec<PrecipitationColumn>,
) {
    out.clear();
    if !camera.iter().all(|value| value.is_finite()) {
        return;
    }
    let top = camera[1] + PRECIPITATION_ABOVE_CAMERA;
    let centre = [camera[0].floor() as i32, camera[2].floor() as i32];
    let radius = PRECIPITATION_RADIUS as f32;
    for dz in -PRECIPITATION_RADIUS..=PRECIPITATION_RADIUS {
        for dx in -PRECIPITATION_RADIUS..=PRECIPITATION_RADIUS {
            let distance = ((dx * dx + dz * dz) as f32).sqrt();
            if distance > radius {
                continue;
            }
            let (x, z) = (centre[0].saturating_add(dx), centre[1].saturating_add(dz));
            let Some(sample) = sampler.sample(x, z) else {
                continue;
            };
            let kind =
                classify_precipitation(sample.temperature, sample.downfall, sample.surface_y);
            let bottom = sample.surface_y as f32;
            if kind == Precipitation::None || bottom >= top {
                continue;
            }
            let alpha = (1.0 - distance / radius) * 0.5 + 0.5;
            out.push(PrecipitationColumn::new(x, z, bottom, kind, alpha));
        }
    }
}

/// Picks where rain lands this tick, for the particle system to spawn splashes; snow never splashes.
pub fn pick_rain_splashes(
    columns: &[PrecipitationColumn],
    level: f32,
    tick: u64,
    out: &mut Vec<[f32; 3]>,
) {
    out.clear();
    let rain: Vec<&PrecipitationColumn> = columns.iter().filter(|c| !c.is_snow()).collect();
    if rain.is_empty() {
        return;
    }
    let count = (unit(level) * MAX_SPLASHES_PER_TICK).round() as u64;
    for index in 0..count {
        let hash = mix64(tick.wrapping_mul(0x9e37_79b9_7f4a_7c15).wrapping_add(index));
        let column = rain[(hash % rain.len() as u64) as usize];
        let fraction = |shift: u32| ((hash >> shift) & 0xffff) as f32 / 65_536.0;
        out.push([
            column.x as f32 + fraction(16),
            column.bottom_y,
            column.z as f32 + fraction(32),
        ]);
    }
}

fn mix64(mut value: u64) -> u64 {
    value ^= value >> 30;
    value = value.wrapping_mul(0xbf58_476d_1ce4_e5b9);
    value ^= value >> 27;
    value = value.wrapping_mul(0x94d0_49bb_1331_11eb);
    value ^ (value >> 31)
}

/// Main-world queue of splash positions for the particle system to drain each frame.
#[derive(Resource, Debug, Default)]
pub struct RainSplashQueue {
    pub positions: Vec<[f32; 3]>,
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Flat {
        surface_y: i32,
        temperature: f32,
        downfall: f32,
    }

    impl ColumnSampler for Flat {
        fn sample(&mut self, _x: i32, _z: i32) -> Option<ColumnSample> {
            Some(ColumnSample {
                surface_y: self.surface_y,
                temperature: self.temperature,
                downfall: self.downfall,
            })
        }
    }

    #[test]
    fn dry_biomes_never_precipitate_and_cold_ones_snow() {
        assert_eq!(classify_precipitation(2.0, 0.0, 64), Precipitation::None);
        assert_eq!(classify_precipitation(0.8, 0.4, 64), Precipitation::Rain);
        assert_eq!(classify_precipitation(0.0, 0.5, 64), Precipitation::Snow);
        assert_eq!(
            classify_precipitation(f32::NAN, 0.5, 64),
            Precipitation::None
        );
    }

    #[test]
    fn high_ground_snows_in_otherwise_rainy_biomes() {
        assert_eq!(classify_precipitation(0.3, 0.5, 64), Precipitation::Rain);
        assert_eq!(classify_precipitation(0.3, 0.5, 200), Precipitation::Snow);
        assert_eq!(altitude_adjusted_temperature(0.5, 10), 0.5);
    }

    #[test]
    fn level_approaches_the_target_without_overshoot() {
        assert_eq!(approach_level(0.0, 1.0, 0.25), 0.25);
        assert_eq!(approach_level(0.9, 1.0, 0.25), 1.0);
        assert!((approach_level(1.0, 0.0, 0.4) - 0.6).abs() < 1.0e-6);
        assert_eq!(approach_level(f32::NAN, 0.5, 1.0), 0.5);
        assert_eq!(approach_level(0.3, 0.8, f32::NAN), 0.3);
    }

    #[test]
    fn column_flags_round_trip() {
        let rain = PrecipitationColumn::new(1, -2, 70.0, Precipitation::Rain, 1.0);
        let snow = PrecipitationColumn::new(1, -2, 70.0, Precipitation::Snow, 0.5);
        assert!(!rain.is_snow() && snow.is_snow());
        assert_eq!(rain.alpha(), 1.0);
        assert!((snow.alpha() - 0.5).abs() < 0.01);
    }

    #[test]
    fn columns_fill_a_disc_and_fade_with_distance() {
        let mut out = Vec::new();
        let mut world = Flat {
            surface_y: 64,
            temperature: 0.8,
            downfall: 0.4,
        };
        build_precipitation_columns(&mut world, [0.5, 70.0, 0.5], &mut out);
        assert!(out.len() > 300 && out.len() <= MAX_PRECIPITATION_COLUMNS);
        let centre = out.iter().find(|c| c.x == 0 && c.z == 0).unwrap();
        let edge = out
            .iter()
            .find(|c| c.x == PRECIPITATION_RADIUS && c.z == 0)
            .unwrap();
        assert_eq!(centre.alpha(), 1.0);
        assert!(edge.alpha() < centre.alpha());
        assert!(out.iter().all(|c| c.bottom_y == 64.0 && !c.is_snow()));
    }

    #[test]
    fn covered_and_dry_columns_are_omitted() {
        let mut out = Vec::new();
        let mut covered = Flat {
            surface_y: 200,
            temperature: 0.8,
            downfall: 0.4,
        };
        build_precipitation_columns(&mut covered, [0.0, 64.0, 0.0], &mut out);
        assert!(out.is_empty(), "surface above the sheet top");
        let mut desert = Flat {
            surface_y: 64,
            temperature: 2.0,
            downfall: 0.0,
        };
        build_precipitation_columns(&mut desert, [0.0, 70.0, 0.0], &mut out);
        assert!(out.is_empty());
        build_precipitation_columns(&mut desert, [f32::NAN, 0.0, 0.0], &mut out);
        assert!(out.is_empty());
    }

    #[test]
    fn splashes_land_on_rain_columns_and_scale_with_level() {
        let columns = [
            PrecipitationColumn::new(4, 5, 70.0, Precipitation::Rain, 1.0),
            PrecipitationColumn::new(9, 9, 70.0, Precipitation::Snow, 1.0),
        ];
        let mut out = Vec::new();
        pick_rain_splashes(&columns, 1.0, 7, &mut out);
        assert_eq!(out.len(), 4);
        for position in &out {
            assert!((4.0..5.0).contains(&position[0]) && (5.0..6.0).contains(&position[2]));
            assert_eq!(position[1], 70.0);
        }
        pick_rain_splashes(&columns, 0.0, 7, &mut out);
        assert!(out.is_empty());
        pick_rain_splashes(&columns[1..], 1.0, 7, &mut out);
        assert!(out.is_empty(), "snow does not splash");
    }

    #[test]
    fn clock_wraps_and_wind_stays_bounded() {
        assert_eq!(precipitation_clock(f64::NAN), 0.0);
        assert!(precipitation_clock(1.0e9) < 4096.0);
        for step in 0..1000 {
            let wind = precipitation_wind(step as f32 * 0.7);
            assert!(wind[0].abs() <= 0.12 && wind[1].abs() <= 0.06);
        }
    }
}
