//! Publishes dropped-item stacks from the world stream as renderer instances.
use std::{collections::HashMap, sync::Arc};

use bevy::{
    ecs::system::SystemParam,
    prelude::{Local, Res, ResMut},
};
use client_world::WorldStream;
use render::{
    DroppedItemInstance, DroppedItemScene, DroppedItemSprite, MAX_ITEM_SPRITE_SIDE,
    MAX_ITEM_SPRITES, dropped_item_transform,
};

use crate::ui_runtime::presentation::UiPresentationRuntime;

/// World size of a sprite's full width; needs independent measurement.
const SPRITE_WORLD_SCALE: f32 = 0.5;
/// Daylight scale until the celestial curve feeds world-space actors.
const DAYLIGHT: f32 = 1.0;

/// Sprites resolved so far, keyed by icon identity so repeated drops share one GPU layer.
#[derive(Default)]
pub(super) struct SpriteCache {
    session: u64,
    revision: u64,
    sprites: Vec<DroppedItemSprite>,
    shared: Arc<[DroppedItemSprite]>,
    index: HashMap<(Arc<str>, u32), u32>,
}

#[derive(SystemParam)]
pub(super) struct DroppedItemPublisher<'w, 's> {
    scene: Option<ResMut<'w, DroppedItemScene>>,
    icons: Option<Res<'w, UiPresentationRuntime>>,
    cache: Local<'s, SpriteCache>,
}

impl DroppedItemPublisher<'_, '_> {
    pub(super) fn publish(&mut self, stream: Option<&WorldStream>, partial_tick: f32) {
        let (Some(scene), Some(icons)) = (self.scene.as_mut(), self.icons.as_ref()) else {
            return;
        };
        let Some(stream) = stream else {
            scene.clear();
            return;
        };
        let cache = &mut *self.cache;
        if cache.session != stream.actor_session_id() {
            *cache = SpriteCache {
                session: stream.actor_session_id(),
                revision: cache.revision.wrapping_add(1),
                ..SpriteCache::default()
            };
        }
        let mut instances = Vec::new();
        for view in stream.dropped_items(partial_tick) {
            let Some(identifier) = view.item.identifier.as_ref() else {
                continue;
            };
            let metadata = view.item.identity.metadata;
            let key = (Arc::clone(identifier), metadata);
            let sprite = match cache.index.get(&key) {
                Some(index) => *index,
                None => {
                    if cache.sprites.len() >= MAX_ITEM_SPRITES {
                        continue;
                    }
                    let Some(pixels) =
                        icons.item_sprite(identifier, metadata, MAX_ITEM_SPRITE_SIDE)
                    else {
                        continue;
                    };
                    let index = cache.sprites.len() as u32;
                    cache.sprites.push(DroppedItemSprite {
                        width: pixels.width,
                        height: pixels.height,
                        rgba8: Arc::from(pixels.rgba8),
                    });
                    cache.shared = Arc::from(cache.sprites.as_slice());
                    cache.revision = cache.revision.wrapping_add(1);
                    cache.index.insert(key, index);
                    index
                }
            };
            let (block_level, sky_level) = stream.light_level_at(view.position);
            for offset in view.copy_offsets.iter().take(usize::from(view.copy_count)) {
                let center = std::array::from_fn(|axis| view.position[axis] + offset[axis]);
                instances.push(DroppedItemInstance {
                    sprite,
                    world_from_item: dropped_item_transform(
                        center,
                        view.yaw_radians,
                        SPRITE_WORLD_SCALE,
                    ),
                    block_level: u32::from(block_level),
                    sky_level: u32::from(sky_level),
                });
            }
        }
        scene.publish(
            cache.revision,
            Arc::clone(&cache.shared),
            &instances,
            DAYLIGHT,
        );
    }
}
