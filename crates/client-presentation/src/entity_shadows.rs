//! Publishes this frame's entity-shadow casters for the renderer.
use bevy::math::Vec3;
use chunk_pipeline::WorldStream;
use render::{ActorCullView, EntityShadowScene};
use render_model::EntityShadow;

use crate::presentation::actors::within_actor_candidate_cube;

/// The local player's caster follows its render-time feet; a spectator casts none.
#[derive(Debug, Clone, Copy)]
pub struct LocalShadowSource {
    pub runtime_id: u64,
    pub feet: Option<[f32; 3]>,
    pub spectator: bool,
}

/// Rebuilds the caster list into `staging` and publishes it; an unchanged list keeps the scene's
/// revision. Casters are admitted as actors are, then culled by their volume.
pub fn publish_entity_shadows(
    stream: Option<&WorldStream>,
    partial_tick: f32,
    local: Option<LocalShadowSource>,
    view: Option<ActorCullView>,
    staging: &mut Vec<EntityShadow>,
    scene: &mut EntityShadowScene,
) {
    staging.clear();
    if let Some(stream) = stream {
        for caster in stream.authority().actor_shadow_casters(partial_tick) {
            let local = local.filter(|local| local.runtime_id == caster.runtime_id);
            if local.is_some_and(|local| local.spectator) {
                continue;
            }
            let feet = local.and_then(|local| local.feet).unwrap_or(caster.feet);
            let shadow = EntityShadow {
                feet,
                radius: caster.radius,
            };
            let player = stream
                .authority()
                .actor(caster.runtime_id)
                .is_some_and(|actor| matches!(actor.kind, protocol::ActorKind::Player { .. }));
            if view.is_none_or(|view| volume_may_be_visible(&shadow, player, view)) {
                staging.push(shadow);
            }
        }
    }
    scene.0.publish(staging);
}

fn volume_may_be_visible(shadow: &EntityShadow, player: bool, view: ActorCullView) -> bool {
    let camera = view.camera_position.to_array();
    if !player && !within_actor_candidate_cube(shadow.feet, camera) {
        return false;
    }
    if Vec3::from_array(shadow.feet).distance_squared(view.camera_position)
        > view.max_distance * view.max_distance
    {
        return false;
    }
    let (low, high) = shadow.bounds();
    let corners: [bevy::math::Vec4; 8] = std::array::from_fn(|index| {
        let pick = |axis: usize| {
            if index >> axis & 1 == 0 {
                low[axis]
            } else {
                high[axis]
            }
        };
        view.clip_from_world * bevy::math::Vec4::new(pick(0), pick(1), pick(2), 1.0)
    });
    let outside = |test: fn(&bevy::math::Vec4) -> bool| corners.iter().all(test);
    !(outside(|clip| clip.x < -clip.w)
        || outside(|clip| clip.x > clip.w)
        || outside(|clip| clip.y < -clip.w)
        || outside(|clip| clip.y > clip.w)
        || outside(|clip| clip.w <= 0.0))
}

#[cfg(test)]
mod tests;
