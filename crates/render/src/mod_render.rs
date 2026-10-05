//! Personal-mod rendering: sandboxed post passes before the HUD and world primitives in the
//! transparent phase. Nothing is queued or drawn while no mod renders.

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

/// One validated pass with its compiled-shader handle once the main world has created it.
#[derive(Clone, Debug)]
pub(crate) struct ScenePass {
    pub(crate) pass: mod_render::Pass,
    pub(crate) shader: Option<Handle<Shader>>,
}

/// The current mod's render output, extracted whenever the mod commits a change.
#[derive(Resource, Clone, Debug, Default)]
pub struct ModRenderScene {
    generation: u64,
    pub(crate) passes: Vec<ScenePass>,
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
    /// Adopts `output` unless `generation` is already applied; unchanged passes keep shaders.
    pub fn apply(&mut self, output: &RenderOutput, generation: u64) {
        if generation == self.generation {
            return;
        }
        self.generation = generation;
        let previous = std::mem::take(&mut self.passes);
        self.passes = output
            .passes
            .iter()
            .map(|pass| ScenePass {
                pass: pass.clone(),
                shader: previous
                    .iter()
                    .find(|old| old.pass.revision == pass.revision)
                    .and_then(|old| old.shader.clone()),
            })
            .collect();
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

/// Creates checked shader assets for newly accepted passes before extraction.
fn create_pass_shaders(mut scene: ResMut<ModRenderScene>, mut shaders: ResMut<Assets<Shader>>) {
    if scene
        .bypass_change_detection()
        .passes
        .iter()
        .all(|pass| pass.shader.is_some())
    {
        return;
    }
    for pass in &mut scene.passes {
        if pass.shader.is_none() {
            pass.shader = Some(shaders.add(crate::shader_safety::from_wgsl(
                pass.pass.shader.to_string(),
                format!("mod-pass-{}-{}.wgsl", pass.pass.name, pass.pass.revision),
            )));
        }
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
    app.add_plugins(ExtractResourcePlugin::<ModRenderScene>::default())
        .add_systems(PostUpdate, create_pass_shaders);
    primitives::install(app);
    app.sub_app_mut(RenderApp).insert_resource(Installed);
    passes::install(app.sub_app_mut(RenderApp));
}
