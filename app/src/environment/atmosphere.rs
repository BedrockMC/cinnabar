//! Per-frame atmosphere derivation: clock and weather into one shared GPU
//! frame, overlaid with client fog profiles and explicit boss requests.

use assets::{BiomeVisualProfile, FogMedium, FogProfile};
use bevy::{
    prelude::{Local, Res, ResMut, Time},
    time::Real,
};
use meshing::CameraMedium;
use render::{AtmosphereFrame, SkyKind, underwater_fog_fraction};
use ui::BossBarView;

use crate::ui_runtime::UiRuntime;

use super::{
    CameraMediumState, EnvironmentContext, EnvironmentProfileRoute, LightningFlashState,
    WeatherDisplay, WeatherState, WorldClock,
    profile_lookup::{dimension_fallback_biome, find_biome_profile},
    visual_world_time,
};

#[must_use]
#[cfg(test)]
pub(crate) fn derive_atmosphere_frame(
    clock: WorldClock,
    weather: WeatherState,
    elapsed_seconds: f64,
) -> AtmosphereFrame {
    derive_atmosphere_frame_for_medium(clock, weather, elapsed_seconds, CameraMedium::Air)
}

#[must_use]
pub(crate) fn derive_atmosphere_frame_for_medium(
    clock: WorldClock,
    weather: WeatherState,
    elapsed_seconds: f64,
    medium: CameraMedium,
) -> AtmosphereFrame {
    AtmosphereFrame::from_bedrock_time(
        visual_world_time(clock, elapsed_seconds),
        weather.rain_level,
        weather.lightning_level,
    )
    .with_camera_medium(medium)
}

/// Clock, weather, dimension sky and biome temperature, before any client profile.
fn derive_base_frame(
    clock: WorldClock,
    weather: WeatherState,
    elapsed_seconds: f64,
    medium: CameraMedium,
    context: &EnvironmentContext,
) -> AtmosphereFrame {
    let cloud_fade = context.render_distance_blocks.unwrap_or(0.0)
        * f32::from(render::CloudRenderConfig::default().distance_scale());
    let frame = derive_atmosphere_frame_for_medium(clock, weather, elapsed_seconds, medium)
        .with_sky_kind(SkyKind::from_dimension(context.dimension))
        .with_cloud_fade_distance(cloud_fade);
    match context.camera_biome_temperature {
        Some(temperature) => frame.with_biome_temperature(temperature),
        None => frame,
    }
}

#[must_use]
pub(crate) fn derive_profiled_atmosphere_frame(
    clock: WorldClock,
    weather: WeatherState,
    elapsed_seconds: f64,
    medium: CameraMedium,
    context: &EnvironmentContext,
    biome_profiles: &[BiomeVisualProfile],
    fog_profiles: &[FogProfile],
) -> (AtmosphereFrame, EnvironmentProfileRoute) {
    let base = derive_base_frame(clock, weather, elapsed_seconds, medium, context);
    let profile = context
        .camera_biome_identifier
        .as_deref()
        .and_then(|identifier| find_biome_profile(biome_profiles, identifier))
        .or_else(|| {
            dimension_fallback_biome(context.dimension)
                .and_then(|identifier| find_biome_profile(biome_profiles, identifier))
        });
    let Some(profile) = profile else {
        return (base, EnvironmentProfileRoute::default());
    };
    let resolve = |requested: FogMedium| {
        let render_distance = context.render_distance_blocks?;
        let fog = fog_profiles
            .binary_search_by(|fog| fog.identifier.cmp(&profile.fog_identifier))
            .ok()
            .map(|index| &fog_profiles[index])?;
        let default_fog = fog_profiles
            .binary_search_by(|fog| fog.identifier.as_ref().cmp("minecraft:fog_default"))
            .ok()
            .map(|index| &fog_profiles[index]);
        fog.distance(requested)
            .or_else(|| default_fog.and_then(|fallback| fallback.distance(requested)))?
            .resolve(render_distance)
    };
    let profiled = base.with_environment_profile(profile.sky_rgb8, None);
    let frame = match medium {
        CameraMedium::Air => {
            profiled.with_blended_fog(resolve(FogMedium::Air), resolve(FogMedium::Weather))
        }
        CameraMedium::Water => profiled.with_environment_profile(None, resolve(FogMedium::Water)),
        CameraMedium::Lava => profiled.with_environment_profile(None, resolve(FogMedium::Lava)),
    };
    (
        frame,
        EnvironmentProfileRoute {
            biome_identifier: Some(profile.biome_identifier.clone()),
            fog_identifier: Some(profile.fog_identifier.clone()),
            atmosphere_identifier: Some(profile.atmosphere_identifier.clone()),
            provisional_lighting_identifier: Some(profile.lighting_identifier.clone()),
        },
    )
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn update_atmosphere_frame(
    clock: Res<WorldClock>,
    weather: Res<WeatherState>,
    medium: Res<CameraMediumState>,
    context: Res<EnvironmentContext>,
    boss_bars: Res<UiRuntime>,
    atmosphere_assets: Res<render::AtmosphereTextureAssets>,
    time: Res<Time<Real>>,
    flash: Res<LightningFlashState>,
    vision: Res<crate::camera::VisionEffects>,
    outputs: (
        ResMut<AtmosphereFrame>,
        ResMut<EnvironmentProfileRoute>,
        ResMut<render::WorldLighting>,
    ),
    settings: Res<crate::settings_runtime::RuntimeSettings>,
    mut display: Local<WeatherDisplay>,
    preferences: (
        Option<Res<crate::menu::MenuRuntime>>,
        Option<ResMut<render::CloudVisibility>>,
    ),
) {
    let (menu, clouds) = preferences;
    let options = menu.as_ref().map(|menu| menu.settings_snapshot().0);
    if let Some(mut clouds) = clouds {
        clouds.0 = options
            .as_ref()
            .is_none_or(|options| options.value("render_clouds") != 0);
    }
    let darkness_scale = options
        .as_ref()
        .map_or(1.0, |options| options.value("darkness") as f32 / 100.0);
    let (mut frame, mut route, mut lighting) = outputs;
    let elapsed = time.elapsed_secs_f64();
    let shown = display.advance(*weather, elapsed);
    let state = derive_boss_environment_iter(boss_bars.boss_bars().stacked_iter());
    let (next_frame, next_route) = match atmosphere_assets.runtime() {
        Some(assets) => derive_profiled_atmosphere_frame(
            *clock,
            shown,
            elapsed,
            medium.0,
            &context,
            assets.biome_profiles(),
            assets.fog_profiles(),
        ),
        None => (
            derive_base_frame(*clock, shown, elapsed, medium.0, &context),
            EnvironmentProfileRoute::default(),
        ),
    };
    let submerged = display.submerged_seconds(medium.0);
    let next_frame = next_frame
        .with_underwater_fog_fraction(underwater_fog_fraction(submerged))
        .with_lightning_flash(flash.level(elapsed))
        .with_vision_effects(
            vision.blindness,
            vision.darkness * darkness_scale,
            vision.night_vision,
        );
    *frame = apply_boss_environment(next_frame, medium.0, state);
    let mut sunrise = frame.sunrise_band();
    for c in &mut sunrise[..3] {
        *c = if *c <= 0.0031308 {
            *c * 12.92
        } else {
            1.055 * c.powf(1.0 / 2.4) - 0.055
        };
    }
    lighting.0 = render::LightmapInputs {
        sky_darken: frame.daylight(),
        sunrise,
        lightning: frame.lightning_flash() > 0.0,
        brightness: settings.user_settings_update().1.video.brightness,
        night_vision: vision.night_vision,
        darkness: vision.darkness * darkness_scale,
        darkness_pulse: render::darkness_pulse(
            visual_world_time(*clock, elapsed) as f32,
            0.0,
            vision.darkness * darkness_scale,
            vision.darkness * darkness_scale,
            0.45,
        ),
        ..Default::default()
    };
    *route = next_route;
}

/// Explicit environment requests retained by active boss bars.
///
/// The pinned protocol-2168 `BossEvent` wire carries no sky-darkening or
/// world-fog fields, so live servers leave both flags unset and this state
/// is inert until a bar explicitly requests an effect.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct BossEnvironmentState {
    pub(crate) darken_sky: bool,
    pub(crate) world_fog: bool,
}

#[cfg(test)]
pub(crate) fn derive_boss_environment(bars: &[BossBarView]) -> BossEnvironmentState {
    derive_boss_environment_iter(bars)
}

/// Derives boss environment flags from an allocation-free boss-bar query.
fn derive_boss_environment_iter<I>(bars: I) -> BossEnvironmentState
where
    I: IntoIterator,
    I::Item: std::borrow::Borrow<BossBarView>,
{
    let mut state = BossEnvironmentState::default();
    for bar in bars {
        let bar = std::borrow::Borrow::borrow(&bar);
        state.darken_sky |= bar.style.darken_sky == Some(true);
        state.world_fog |= bar.style.create_world_fog == Some(true);
    }
    state
}

/// Boss effects respond only in air; water and lava media own their fog
/// completely and must not be overridden by a boss flag.
pub(crate) fn apply_boss_environment(
    frame: AtmosphereFrame,
    medium: CameraMedium,
    state: BossEnvironmentState,
) -> AtmosphereFrame {
    match medium {
        CameraMedium::Air => frame.with_boss_environment(state.darken_sky, state.world_fog),
        CameraMedium::Water | CameraMedium::Lava => frame,
    }
}
