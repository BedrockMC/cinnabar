//! Opt-in non-parity rendering. Vanilla cameras never enable these effects.

use bevy::{
    prelude::*,
    render::extract_component::{ExtractComponent, ExtractComponentPlugin},
};

/// Per-camera opt-in for the Enhanced render mode and its quality knobs.
#[derive(Component, ExtractComponent, Clone, Copy, Debug, PartialEq)]
pub struct EnhancedRendering {
    pub shadows: bool,
    pub shadow_resolution: u32,
    /// Sun shadow cascades, clamped to `1..=MAX_SHADOW_CASCADES`.
    pub shadow_cascades: u32,
    /// Blocks from the camera covered by the last cascade.
    pub shadow_distance: f32,
    pub bloom: bool,
    pub light_shafts: bool,
    pub waving: bool,
    pub water_reflections: bool,
}

pub const MAX_SHADOW_CASCADES: u32 = 3;

impl Default for EnhancedRendering {
    fn default() -> Self {
        Self {
            shadows: false,
            shadow_resolution: 1024,
            shadow_cascades: 2,
            shadow_distance: 96.0,
            bloom: true,
            light_shafts: true,
            waving: true,
            water_reflections: true,
        }
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct EnhancedRenderPlugin;

impl Plugin for EnhancedRenderPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(ExtractComponentPlugin::<EnhancedRendering>::default());
    }
}
