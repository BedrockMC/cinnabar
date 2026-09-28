//! Pure sky-model math: celestial angle, sky light, sky colour, stars, sunrise band.
//!
//! Angles are in turns with 0 at noon, 0.25 at sunset, 0.5 at midnight. Constants are
//! observed vanilla behaviour and need native-capture calibration before acceptance.

use std::f32::consts::{PI, TAU};

/// Ticks in one Bedrock day.
pub const DAY_TICKS: f64 = 24_000.0;

/// Sky-light transfer at night: brightness-ramp level 4, i.e. 15 minus the 11-level night reduction.
pub const NIGHT_SKY_TRANSFER: f32 = 0.083_333_336;

const RAIN_LIGHT_LOSS: f32 = 5.0 / 16.0;
const SUNRISE_HALF_WIDTH: f32 = 0.4;

/// Eased celestial angle for an absolute tick; the sun rises slowly and sets slowly.
#[must_use]
pub fn celestial_angle(absolute_ticks: f64) -> f32 {
    let ticks = if absolute_ticks.is_finite() {
        absolute_ticks
    } else {
        0.0
    };
    let linear = (ticks.rem_euclid(DAY_TICKS) / DAY_TICKS - 0.25).rem_euclid(1.0) as f32;
    let eased = 0.5 - (linear * PI).cos() * 0.5;
    linear + (eased - linear) / 3.0
}

/// Cosine of the sun's zenith angle: 1 at noon, -1 at midnight.
#[must_use]
pub fn sun_height(angle: f32) -> f32 {
    (angle * TAU).cos()
}

/// Sun position on the unit sphere; east is +x, and noon is straight up.
#[must_use]
pub fn sun_direction(angle: f32) -> [f32; 3] {
    let (sin, cos) = (angle * TAU).sin_cos();
    [clean_unit(-sin), clean_unit(cos), 0.0]
}

/// Day plateau: 1 while the sun is high, 0 at night, with a short ramp around the horizon.
#[must_use]
pub fn day_plateau(angle: f32) -> f32 {
    (sun_height(angle) * 2.0 + 0.5).clamp(0.0, 1.0)
}

/// Sky-light transfer applied to the sky channel of the lightmap, in `NIGHT_SKY_TRANSFER..=1`.
#[must_use]
pub fn daylight(angle: f32, rain: f32, thunder: f32) -> f32 {
    let storm = (1.0 - unit(rain) * RAIN_LIGHT_LOSS) * (1.0 - unit(thunder) * RAIN_LIGHT_LOSS);
    lerp(NIGHT_SKY_TRANSFER, 1.0, day_plateau(angle) * storm)
}

/// Multiplier applied to the fog colour: fog never fully blackens.
#[must_use]
pub fn fog_brightness(angle: f32) -> f32 {
    lerp(0.06, 1.0, day_plateau(angle))
}

/// Star alpha: zero by day, at most 0.5 at midnight, hidden by rain.
#[must_use]
pub fn star_brightness(angle: f32, rain: f32) -> f32 {
    let base = (1.0 - (sun_height(angle) * 2.0 + 0.25)).clamp(0.0, 1.0);
    base * base * 0.5 * (1.0 - unit(rain))
}

/// Sunrise/sunset glow as `[r, g, b, alpha]` in gamma space; alpha is zero outside the band.
#[must_use]
pub fn sunrise_band(angle: f32, rain: f32) -> [f32; 4] {
    let height = sun_height(angle);
    if !(-SUNRISE_HALF_WIDTH..=SUNRISE_HALF_WIDTH).contains(&height) {
        return [0.0; 4];
    }
    let f = height / SUNRISE_HALF_WIDTH * 0.5 + 0.5;
    let alpha = 1.0 - (1.0 - (f * PI).sin()) * 0.99;
    [
        f * 0.3 + 0.7,
        f * f * 0.7 + 0.2,
        0.2,
        alpha * alpha * (1.0 - unit(rain)),
    ]
}

/// Clear-sky colour (gamma space) derived from biome temperature.
#[must_use]
pub fn sky_colour_for_temperature(temperature: f32) -> [f32; 3] {
    let t = if temperature.is_finite() {
        (temperature / 3.0).clamp(-1.0, 1.0)
    } else {
        0.0
    };
    hsv_to_rgb(0.622_222_2 - t * 0.05, 0.5 + t * 0.1, 1.0)
}

/// Desaturates and darkens a gamma-space sky or fog colour for rain and thunder.
#[must_use]
pub fn storm_tint(colour: [f32; 3], rain: f32, thunder: f32) -> [f32; 3] {
    let mut colour = colour;
    for (amount, grey_scale) in [(unit(rain) * 0.75, 0.6), (unit(thunder) * 0.75, 0.2)] {
        if amount > 0.0 {
            let grey = (colour[0] * 0.3 + colour[1] * 0.59 + colour[2] * 0.11) * grey_scale;
            colour = colour.map(|channel| lerp(channel, grey, amount));
        }
    }
    colour
}

/// Underwater fog reach as a fraction of the profile end distance after `seconds` submerged.
#[must_use]
pub fn underwater_fog_fraction(seconds: f32) -> f32 {
    const MIN_PERCENT: f32 = 0.25;
    const MID: (f32, f32) = (5.0, 0.6);
    const MAX: (f32, f32) = (30.0, 1.0);
    let seconds = if seconds.is_finite() {
        seconds.max(0.0)
    } else {
        0.0
    };
    if seconds <= MID.0 {
        lerp(MIN_PERCENT, MID.1, seconds / MID.0)
    } else if seconds < MAX.0 {
        lerp(MID.1, MAX.1, (seconds - MID.0) / (MAX.0 - MID.0))
    } else {
        MAX.1
    }
}

pub(crate) fn srgb_to_linear(value: f32) -> f32 {
    let value = value.clamp(0.0, 1.0);
    if value <= 0.040_45 {
        value / 12.92
    } else {
        ((value + 0.055) / 1.055).powf(2.4)
    }
}

pub(crate) fn rgb_to_linear(rgb: [f32; 3]) -> [f32; 3] {
    rgb.map(srgb_to_linear)
}

pub(crate) fn unit(value: f32) -> f32 {
    if value.is_finite() {
        value.clamp(0.0, 1.0)
    } else {
        0.0
    }
}

/// Exact at both endpoints, unlike `left + (right - left) * amount`.
pub(crate) fn lerp(left: f32, right: f32, amount: f32) -> f32 {
    left * (1.0 - amount) + right * amount
}

fn clean_unit(value: f32) -> f32 {
    if value.abs() < 1.0e-6 { 0.0 } else { value }
}

fn hsv_to_rgb(hue: f32, saturation: f32, value: f32) -> [f32; 3] {
    let hue = hue.rem_euclid(1.0) * 6.0;
    let sector = hue.floor();
    let fraction = hue - sector;
    let p = value * (1.0 - saturation);
    let q = value * (1.0 - saturation * fraction);
    let t = value * (1.0 - saturation * (1.0 - fraction));
    match sector as u32 % 6 {
        0 => [value, t, p],
        1 => [q, value, p],
        2 => [p, value, t],
        3 => [p, q, value],
        4 => [t, p, value],
        _ => [value, p, q],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn angle_is_zero_at_noon_and_half_at_midnight() {
        assert!(celestial_angle(6_000.0).abs() < 1.0e-6);
        assert!((celestial_angle(18_000.0) - 0.5).abs() < 1.0e-6);
        assert!((celestial_angle(0.0) - celestial_angle(24_000.0)).abs() < 1.0e-6);
    }

    #[test]
    fn angle_easing_slows_the_sun_near_noon_and_speeds_it_at_the_horizon() {
        // Ten percent of a quarter day from noon covers less than the linear angle.
        let after_noon = celestial_angle(6_000.0 + 600.0);
        assert!(after_noon < 600.0 / DAY_TICKS as f32);
        assert!(after_noon > 0.0);
    }

    #[test]
    fn angle_is_monotonic_across_the_day() {
        let mut previous = celestial_angle(6_000.0);
        for tick in (6_100..30_000).step_by(100) {
            let angle = celestial_angle(f64::from(tick));
            assert!(angle >= previous - 1.0e-6, "tick {tick}");
            previous = angle;
        }
    }

    #[test]
    fn sun_is_overhead_at_noon_and_below_at_midnight() {
        assert_eq!(sun_direction(celestial_angle(6_000.0)), [0.0, 1.0, 0.0]);
        assert!(sun_direction(celestial_angle(18_000.0))[1] < -0.999);
    }

    #[test]
    fn sun_rises_in_the_east_and_sets_in_the_west() {
        assert!(sun_direction(celestial_angle(0.0))[0] > 0.9);
        assert!(sun_direction(celestial_angle(12_000.0))[0] < -0.9);
    }

    #[test]
    fn daylight_has_a_day_plateau_and_a_night_floor() {
        for tick in [3_000.0, 6_000.0, 9_000.0] {
            let level = daylight(celestial_angle(tick), 0.0, 0.0);
            assert!((level - 1.0).abs() < 1.0e-6, "tick {tick}");
        }
        for tick in [15_000.0, 18_000.0, 21_000.0] {
            assert_eq!(
                daylight(celestial_angle(tick), 0.0, 0.0),
                NIGHT_SKY_TRANSFER,
                "tick {tick}"
            );
        }
        let dusk = daylight(celestial_angle(12_000.0), 0.0, 0.0);
        assert!(dusk > NIGHT_SKY_TRANSFER && dusk < 1.0);
    }

    #[test]
    fn rain_and_thunder_dim_only_the_lit_portion() {
        let noon = celestial_angle(6_000.0);
        let rainy = daylight(noon, 1.0, 0.0);
        let stormy = daylight(noon, 1.0, 1.0);
        assert!(rainy < 1.0 && stormy < rainy);
        assert_eq!(
            daylight(celestial_angle(18_000.0), 1.0, 1.0),
            NIGHT_SKY_TRANSFER
        );
        assert!((daylight(noon, f32::NAN, f32::INFINITY) - 1.0).abs() < 1.0e-6);
    }

    #[test]
    fn stars_appear_only_at_night_and_fade_in_rain() {
        assert_eq!(star_brightness(celestial_angle(6_000.0), 0.0), 0.0);
        let midnight = star_brightness(celestial_angle(18_000.0), 0.0);
        assert!((midnight - 0.5).abs() < 1.0e-6);
        assert_eq!(star_brightness(celestial_angle(18_000.0), 1.0), 0.0);
    }

    #[test]
    fn sunrise_band_exists_only_near_the_horizon() {
        assert_eq!(sunrise_band(celestial_angle(6_000.0), 0.0)[3], 0.0);
        assert_eq!(sunrise_band(celestial_angle(18_000.0), 0.0)[3], 0.0);
        let sunrise = sunrise_band(celestial_angle(0.0), 0.0);
        assert!(sunrise[3] > 0.9 && sunrise[0] > sunrise[1] && sunrise[1] > sunrise[2]);
        assert_eq!(sunrise_band(celestial_angle(0.0), 1.0)[3], 0.0);
    }

    #[test]
    fn temperate_sky_is_the_familiar_light_blue() {
        let [r, g, b] = sky_colour_for_temperature(0.8);
        assert!((r * 255.0 - 120.0).abs() < 4.0, "{r}");
        assert!((g * 255.0 - 167.0).abs() < 4.0, "{g}");
        assert!((b - 1.0).abs() < 1.0e-6);
        assert_ne!(
            sky_colour_for_temperature(-1.0),
            sky_colour_for_temperature(2.0)
        );
        assert_eq!(
            sky_colour_for_temperature(f32::NAN),
            sky_colour_for_temperature(0.0)
        );
    }

    #[test]
    fn storm_tint_is_identity_when_clear_and_darkens_when_stormy() {
        let colour = [0.47, 0.65, 1.0];
        assert_eq!(storm_tint(colour, 0.0, 0.0), colour);
        let rainy = storm_tint(colour, 1.0, 0.0);
        let stormy = storm_tint(colour, 1.0, 1.0);
        assert!(rainy[2] < colour[2] && stormy[2] < rainy[2]);
    }

    #[test]
    fn underwater_fog_reach_grows_to_full_over_thirty_seconds() {
        assert_eq!(underwater_fog_fraction(0.0), 0.25);
        assert!((underwater_fog_fraction(5.0) - 0.6).abs() < 1.0e-6);
        assert_eq!(underwater_fog_fraction(30.0), 1.0);
        assert_eq!(underwater_fog_fraction(1.0e6), 1.0);
        assert_eq!(underwater_fog_fraction(f32::NAN), 0.25);
    }
}
