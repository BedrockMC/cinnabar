//! Personal-mod rendering: sandboxed post passes before the HUD and world primitives in the
//! transparent phase. Nothing is queued or drawn while no mod renders. Pass pipelines are
//! owned per pass revision, so replaced or reloaded passes release them.

mod passes;
mod primitives;
#[cfg(test)]
mod tests;

use bevy::{
    prelude::*,
    render::{
        RenderApp, extract_resource::ExtractResource, extract_resource::ExtractResourcePlugin,
    },
};
use mod_render::{RenderOutput, geometry::ModVertex};
use std::sync::Arc;

pub use passes::ModPassLabel;

/// The current mod's render output, extracted whenever the mod commits a change.
#[derive(Resource, Clone, Debug, Default)]
pub struct ModRenderScene {
    generation: u64,
    pub(crate) passes: Vec<mod_render::Pass>,
    pub(crate) vertices: Arc<[ModVertex]>,
    primitives: Arc<mod_render::Primitives>,
}

impl ExtractResource for ModRenderScene {
    type Source = Self;

    fn extract_resource(source: &Self) -> Self {
        source.clone()
    }
}

impl ModRenderScene {
    /// Adopts `output` unless `generation` is already applied.
    pub fn apply(&mut self, output: &RenderOutput, generation: u64) {
        if generation == self.generation {
            return;
        }
        self.generation = generation;
        self.passes.clone_from(&output.passes);
        if !Arc::ptr_eq(&self.primitives, &output.primitives) {
            self.primitives = Arc::clone(&output.primitives);
            self.vertices = mod_render::geometry::build(&output.primitives).into();
        }
    }

    /// Drops every pass and primitive, as when a mod traps, reloads or is revoked.
    pub fn clear(&mut self) {
        if self.generation != 0 || !self.passes.is_empty() || !self.vertices.is_empty() {
            *self = Self::default();
        }
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }

    pub fn pass_count(&self) -> usize {
        self.passes.len()
    }

    pub fn vertex_count(&self) -> usize {
        self.vertices.len()
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct ModRenderPlugin;

impl Plugin for ModRenderPlugin {
    fn build(&self, app: &mut App) {
        install(app);
    }

    fn finish(&self, app: &mut App) {
        install(app);
    }
}

#[derive(Resource)]
struct Installed;

fn install(app: &mut App) {
    app.init_resource::<ModRenderScene>();
    let Some(render_app) = app.get_sub_app(RenderApp) else {
        return;
    };
    if render_app.world().contains_resource::<Installed>() {
        passes::install_graph(app.sub_app_mut(RenderApp).world_mut());
        return;
    }
    app.add_plugins(ExtractResourcePlugin::<ModRenderScene>::default());
    primitives::install(app);
    app.sub_app_mut(RenderApp).insert_resource(Installed);
    passes::install(app.sub_app_mut(RenderApp));
}
