//! Hands committed mod render output to the renderer; depth sampling follows its grant.

use crate::camera::FlyCamera;
use bevy::{prelude::*, render::render_resource::TextureUsages};
use render::ModRenderScene;

/// Republishes only a new generation, so frames without changes extract nothing.
pub(super) fn publish(scene: Option<ResMut<ModRenderScene>>, host: &mod_host::ModHost) {
    let Some(mut scene) = scene else { return };
    let (output, generation) = host.render();
    if scene.generation() != generation {
        scene.apply(output, generation);
    }
}

/// Scene depth becomes sampleable once a mod holds both render grants.
pub(super) fn grant_depth_sampling(
    runtime: Option<Res<super::ModRuntime>>,
    mut cameras: Query<&mut Camera3d, With<FlyCamera>>,
) {
    if !runtime.is_some_and(|runtime| runtime.grants.render && runtime.grants.render_depth) {
        return;
    }
    for mut camera in &mut cameras {
        let usage = TextureUsages::from(camera.depth_texture_usages);
        if !usage.contains(TextureUsages::TEXTURE_BINDING) {
            camera.depth_texture_usages = (usage | TextureUsages::TEXTURE_BINDING).into();
        }
    }
}
