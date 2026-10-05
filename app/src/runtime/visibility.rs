use std::collections::HashMap;

use bevy::prelude::{
    Add, Entity, On, Query, Remove, Res, ResMut, Resource, Transform, Visibility, With,
};
use chunk_pipeline::CaveVisibleSet;
use render::{ChunkRenderInstance, RuntimeStage, RuntimeStageProfiler};
use world::SubChunkKey;

use crate::{
    camera::FlyCamera,
    runtime::{telemetry::camera_sub_chunk_key, world::ClientWorld},
};
use diagnostics::metrics::{DiagnosticQuadTracker, MetricsCollector};

#[derive(Resource, Default)]
pub(crate) struct CaveVisibilityCache {
    pub(crate) camera: Option<SubChunkKey>,
    pub(crate) graph_generation: Option<u64>,
    pub(crate) visible: CaveVisibleSet,
    next_visible: CaveVisibleSet,
    scratch: chunk_pipeline::CaveVisibilityScratch,
    pub(crate) rendered: HashMap<SubChunkKey, Entity>,
    pub(crate) visible_rendered: usize,
    pub(crate) initialized: bool,
}

impl CaveVisibilityCache {
    pub(crate) fn is_visible(&self, key: SubChunkKey) -> bool {
        !self.initialized || self.visible.contains(&key)
    }

    /// Adopts `next_visible`, calling `set` only for rendered entities whose visibility flips.
    fn publish_next(&mut self, mut set: impl FnMut(Entity, bool)) {
        std::mem::swap(&mut self.visible, &mut self.next_visible);
        if !std::mem::replace(&mut self.initialized, true) {
            // Everything counted as visible until the first result.
            self.visible_rendered = 0;
            for (key, &entity) in &self.rendered {
                let visible = self.visible.contains(key);
                if !visible {
                    set(entity, false);
                }
                self.visible_rendered += usize::from(visible);
            }
            return;
        }
        let (previous, current) = (&self.next_visible, &self.visible);
        for (key, visible) in previous
            .iter()
            .filter(|key| !current.contains(key))
            .map(|key| (key, false))
            .chain(
                current
                    .iter()
                    .filter(|key| !previous.contains(key))
                    .map(|key| (key, true)),
            )
        {
            if let Some(&entity) = self.rendered.get(&key) {
                set(entity, visible);
                if visible {
                    self.visible_rendered += 1;
                } else {
                    self.visible_rendered = self.visible_rendered.saturating_sub(1);
                }
            }
        }
    }

    /// Whether the culler hides the box from `low` to `high` in `dimension`: as vanilla's
    /// `isAABBVisible`, only when the cache matches graph `generation` and every sub-chunk the
    /// box overlaps is `known` to that graph without being visible.
    pub(crate) fn hides_box(
        &self,
        dimension: i32,
        generation: u64,
        known: impl Fn(SubChunkKey) -> bool,
        low: [f32; 3],
        high: [f32; 3],
    ) -> bool {
        if !self.initialized
            || self
                .camera
                .is_none_or(|camera| camera.dimension != dimension)
            || self.graph_generation != Some(generation)
            || low.iter().chain(&high).any(|value| !value.is_finite())
        {
            return false;
        }
        let section = |value: f32| (value.floor() as i32).div_euclid(16);
        for x in section(low[0])..=section(high[0]) {
            for y in section(low[1])..=section(high[1]) {
                for z in section(low[2])..=section(high[2]) {
                    let key = SubChunkKey::new(dimension, x, y, z);
                    if !known(key) || self.visible.contains(&key) {
                        return false;
                    }
                }
            }
        }
        true
    }
}

#[derive(Resource)]
pub(crate) struct AppMetrics(pub(crate) MetricsCollector);

#[derive(Resource, Default)]
pub(crate) struct DiagnosticQuads(pub(crate) DiagnosticQuadTracker);

pub(crate) fn refresh_cave_visibility(
    client_world: Res<ClientWorld>,
    camera: Query<&Transform, With<FlyCamera>>,
    mut cache: ResMut<CaveVisibilityCache>,
    mut chunks: Query<&mut Visibility, With<ChunkRenderInstance>>,
    profiler: Option<Res<RuntimeStageProfiler>>,
) {
    let _timer = profiler
        .as_deref()
        .map(|profiler| profiler.time(RuntimeStage::CaveVisibility));
    let (Some(stream), Ok(camera)) = (client_world.stream.as_ref(), camera.single()) else {
        return;
    };
    let camera_key = camera_sub_chunk_key(stream.current_dimension(), camera.translation);
    let generation = stream.connectivity_generation();
    if cache.camera == Some(camera_key)
        && cache.graph_generation == Some(generation)
        && cache.initialized
    {
        return;
    }

    let cache = &mut *cache;
    stream.cave_visible_sub_chunks_into(camera_key, &mut cache.scratch, &mut cache.next_visible);
    cache.camera = Some(camera_key);
    cache.graph_generation = Some(generation);
    if cache.initialized && cache.visible == cache.next_visible {
        return;
    }
    cache.publish_next(|entity, visible| {
        let Ok(mut visibility) = chunks.get_mut(entity) else {
            return;
        };
        let desired = if visible {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        };
        if *visibility != desired {
            *visibility = desired;
        }
    });
}

pub(crate) fn apply_added_chunk_visibility(
    add: On<Add, ChunkRenderInstance>,
    mut cache: ResMut<CaveVisibilityCache>,
    mut chunks: Query<(&ChunkRenderInstance, &mut Visibility)>,
) {
    let Ok((instance, mut visibility)) = chunks.get_mut(add.entity) else {
        return;
    };
    let key = instance.key();
    let is_visible = cache.is_visible(key);
    *visibility = if is_visible {
        Visibility::Inherited
    } else {
        Visibility::Hidden
    };
    if cache.rendered.insert(key, add.entity).is_none() && is_visible {
        cache.visible_rendered += 1;
    }
}

pub(crate) fn remove_chunk_visibility(
    remove: On<Remove, ChunkRenderInstance>,
    mut cache: ResMut<CaveVisibilityCache>,
    chunks: Query<&ChunkRenderInstance>,
) {
    let Ok(instance) = chunks.get(remove.entity) else {
        return;
    };
    let key = instance.key();
    // A replacement entity at the same key may already own the slot.
    if cache.rendered.get(&key) == Some(&remove.entity)
        && cache.rendered.remove(&key).is_some()
        && cache.is_visible(key)
    {
        cache.visible_rendered = cache.visible_rendered.saturating_sub(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An actor is hidden only when every sub-chunk its box touches is known and not visible.
    #[test]
    fn a_box_is_hidden_only_when_all_its_known_sub_chunks_are_invisible() {
        let key = |x, y, z| SubChunkKey::new(0, x, y, z);
        let cache = CaveVisibilityCache {
            camera: Some(key(0, 4, 0)),
            graph_generation: Some(7),
            visible: [key(1, 4, 0)].into_iter().collect(),
            initialized: true,
            ..CaveVisibilityCache::default()
        };
        let known = |key: SubChunkKey| key.y < 8;
        let hides = |low, high| cache.hides_box(0, 7, known, low, high);
        assert!(hides([-8.0, 64.0, 4.0], [-7.0, 66.0, 5.0]));
        // Straddling into the visible neighbour, or reaching an unknown sub-chunk, draws it.
        assert!(!hides([15.5, 64.0, 4.0], [16.5, 66.0, 5.0]));
        assert!(!hides([-8.0, 127.0, 4.0], [-7.0, 129.0, 5.0]));
        // A stale graph or another dimension never hides anything.
        assert!(!cache.hides_box(0, 8, known, [-8.0, 64.0, 4.0], [-7.0, 66.0, 5.0]));
        assert!(!cache.hides_box(1, 7, known, [-8.0, 64.0, 4.0], [-7.0, 66.0, 5.0]));
    }

    /// Only entities whose key entered or left the visible set are written.
    #[test]
    fn publishing_touches_only_entities_whose_visibility_flipped() {
        let key = |x| SubChunkKey::new(0, x, 0, 0);
        let entity = |x| Entity::from_raw_u32(x as u32 + 1).unwrap();
        let mut cache = CaveVisibilityCache {
            rendered: (0..4).map(|x| (key(x), entity(x))).collect(),
            visible_rendered: 4,
            ..CaveVisibilityCache::default()
        };
        let mut writes = Vec::new();
        cache.next_visible = [key(0), key(1), key(9)].into_iter().collect();
        cache.publish_next(|entity, visible| writes.push((entity, visible)));
        writes.sort_by_key(|(entity, _)| entity.index());
        assert_eq!(writes, [(entity(2), false), (entity(3), false)]);
        assert_eq!(cache.visible_rendered, 2);

        writes.clear();
        cache.next_visible = [key(1), key(2), key(9)].into_iter().collect();
        cache.publish_next(|entity, visible| writes.push((entity, visible)));
        writes.sort_by_key(|(entity, _)| entity.index());
        assert_eq!(writes, [(entity(0), false), (entity(2), true)]);
        assert_eq!(cache.visible_rendered, 2);

        writes.clear();
        cache.next_visible = cache.visible.clone();
        cache.publish_next(|entity, visible| writes.push((entity, visible)));
        assert!(writes.is_empty());
    }
}
