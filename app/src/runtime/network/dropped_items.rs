//! Publishes dropped items, falling blocks, primed TNT and ropes as renderer geometry.
use std::{collections::HashMap, sync::Arc};

use assets::{
    BlockFace, ItemVisualRoute, MATERIAL_FLAG_FOLIAGE_TINT, MATERIAL_FLAG_GRASS_TINT,
    MATERIAL_FLAG_TINT_MASK, MATERIAL_FLAG_WATER_TINT, NetworkIdMode, RuntimeAssets, VisualKind,
};
use bevy::{
    ecs::system::SystemParam,
    prelude::{Local, Res, ResMut},
};
use client_world::{BlockEntityKind, RopeKind, WorldStream};
use render::{
    ChunkTextureAssets, DroppedItemCube, DroppedItemInstance, DroppedItemModel, DroppedItemScene,
    DroppedItemSprite, ItemMeshVertex, MAX_ITEM_LAYERS, MAX_ITEM_SPRITE_SIDE,
    dropped_item_transform, pack_overlay_rgba8, rope_color, rope_ribbon,
};

use crate::ui_runtime::presentation::UiPresentationRuntime;

// Provisional world sizes and colours; each needs independent measurement.
const SPRITE_WORLD_SCALE: f32 = 0.5;
const DROPPED_BLOCK_SCALE: f32 = 0.25;
const FALLING_BLOCK_SCALE: f32 = 0.98;
const TNT_FLASH_OVERLAY: [f32; 4] = [1.0, 1.0, 1.0, 0.8];
const FISHING_SEGMENTS: usize = 16;
const FISHING_SAG_FRACTION: f32 = 0.1;
const FISHING_HALF_WIDTH: f32 = 0.006;
const LEAD_SEGMENTS: usize = 16;
const LEAD_SAG_FRACTION: f32 = 0.12;
const LEAD_HALF_WIDTH: f32 = 0.0125;
const GRASS_TINT_RGB: [u8; 3] = [0x79, 0xc0, 0x5a];
const FOLIAGE_TINT_RGB: [u8; 3] = [0x77, 0xab, 0x2f];
const WATER_TINT_RGB: [u8; 3] = [0x3f, 0x76, 0xe4];
/// Daylight scale until the celestial curve feeds world-space actors.
const DAYLIGHT: f32 = 1.0;

#[derive(Clone, PartialEq, Eq, Hash)]
enum ModelKey {
    Icon(Arc<str>, u32),
    Block { hashed: bool, id: u32 },
}

/// Models resolved so far; failed block lookups are remembered so they are not rebuilt per frame.
#[derive(Default)]
pub(super) struct ModelCache {
    session: u64,
    assets: Option<render::ChunkTextureAssetIdentity>,
    revision: u64,
    models: Vec<DroppedItemModel>,
    layers: usize,
    shared: Arc<[DroppedItemModel]>,
    index: HashMap<ModelKey, Option<u32>>,
}

impl ModelCache {
    fn insert(&mut self, key: ModelKey, model: Option<DroppedItemModel>) -> Option<u32> {
        let cost = match &model {
            Some(DroppedItemModel::Cube(_)) => 6,
            Some(DroppedItemModel::Sprite(_)) => 1,
            None => 0,
        };
        let index = model.and_then(|model| {
            (self.layers + cost < MAX_ITEM_LAYERS).then(|| {
                self.layers += cost;
                self.models.push(model);
                self.shared = Arc::from(self.models.as_slice());
                self.revision = self.revision.wrapping_add(1);
                (self.models.len() - 1) as u32
            })
        });
        self.index.insert(key, index);
        index
    }
}

#[derive(SystemParam)]
pub(super) struct DroppedItemPublisher<'w, 's> {
    scene: Option<ResMut<'w, DroppedItemScene>>,
    icons: Option<Res<'w, UiPresentationRuntime>>,
    textures: Option<Res<'w, ChunkTextureAssets>>,
    cache: Local<'s, ModelCache>,
}

fn tint_rgba(flags: u32) -> u32 {
    let [r, g, b] = match flags & MATERIAL_FLAG_TINT_MASK {
        MATERIAL_FLAG_GRASS_TINT => GRASS_TINT_RGB,
        MATERIAL_FLAG_FOLIAGE_TINT => FOLIAGE_TINT_RGB,
        MATERIAL_FLAG_WATER_TINT => WATER_TINT_RGB,
        _ => [255; 3],
    };
    rope_color(r, g, b)
}

/// Builds a unit cube from a cube-kind block's six face textures, or `None` for other kinds.
fn block_cube(assets: &RuntimeAssets, mode: NetworkIdMode, id: u32) -> Option<DroppedItemCube> {
    let block = assets.resolve(mode, id);
    if !block.is_known() || block.kind() != VisualKind::Cube {
        return None;
    }
    let mut tile_size = None;
    let mut faces: Vec<Arc<[u8]>> = Vec::with_capacity(6);
    let mut tints = [0_u32; 6];
    for (index, face) in BlockFace::ALL.into_iter().enumerate() {
        let material = assets.material(block.face(face).material_id());
        let page = assets
            .texture_pages()
            .get(material.texture.page() as usize)?;
        let mip = page.texture.mips.first()?;
        let size = mip.size;
        if size == 0 || size > MAX_ITEM_SPRITE_SIDE || *tile_size.get_or_insert(size) != size {
            return None;
        }
        let bytes = (size * size * 4) as usize;
        let start = material.texture.layer() as usize * bytes;
        faces.push(Arc::from(mip.rgba8.get(start..start + bytes)?));
        tints[index] = tint_rgba(material.flags);
    }
    Some(DroppedItemCube {
        tile: tile_size?,
        faces: faces.try_into().ok()?,
        tints,
    })
}

impl DroppedItemPublisher<'_, '_> {
    fn block_model(
        cache: &mut ModelCache,
        assets: &RuntimeAssets,
        mode: NetworkIdMode,
        id: u32,
    ) -> Option<u32> {
        let key = ModelKey::Block {
            hashed: mode == NetworkIdMode::Hashed,
            id,
        };
        if let Some(cached) = cache.index.get(&key) {
            return *cached;
        }
        let model = block_cube(assets, mode, id).map(DroppedItemModel::Cube);
        cache.insert(key, model)
    }

    fn icon_model(
        cache: &mut ModelCache,
        icons: &UiPresentationRuntime,
        identifier: &Arc<str>,
        metadata: u32,
    ) -> Option<u32> {
        let key = ModelKey::Icon(Arc::clone(identifier), metadata);
        if let Some(cached) = cache.index.get(&key) {
            return *cached;
        }
        // An icon that is not ready yet is retried next frame rather than cached as missing.
        let pixels = icons.item_sprite(identifier, metadata, MAX_ITEM_SPRITE_SIDE)?;
        let model = DroppedItemModel::Sprite(DroppedItemSprite {
            width: pixels.width,
            height: pixels.height,
            rgba8: Arc::from(pixels.rgba8),
        });
        cache.insert(key, Some(model))
    }

    pub(super) fn publish(
        &mut self,
        stream: Option<&WorldStream>,
        camera: Option<[f32; 3]>,
        partial_tick: f32,
    ) {
        let (Some(scene), Some(icons)) = (self.scene.as_mut(), self.icons.as_ref()) else {
            return;
        };
        let Some(stream) = stream else {
            scene.clear();
            return;
        };
        let cache = &mut *self.cache;
        let assets_identity = self.textures.as_ref().map(|textures| textures.identity());
        if cache.session != stream.actor_session_id() || cache.assets != assets_identity {
            *cache = ModelCache {
                session: stream.actor_session_id(),
                assets: assets_identity,
                revision: cache.revision.wrapping_add(1),
                ..ModelCache::default()
            };
        }
        let assets = self.textures.as_ref().map(|textures| textures.assets());
        let mode = stream.network_id_mode();
        let mut instances = Vec::new();

        for view in stream.dropped_items(partial_tick) {
            let Some(identifier) = view.item.identifier.as_ref() else {
                continue;
            };
            let block_id = match view.item.visual {
                ItemVisualRoute::BlockItem(id) => Some((NetworkIdMode::Sequential, id.0)),
                ItemVisualRoute::RetainedBlock { block_runtime_id } => {
                    u32::try_from(block_runtime_id).ok().map(|id| (mode, id))
                }
                _ => None,
            };
            let cube = block_id
                .zip(assets)
                .and_then(|((mode, id), assets)| Self::block_model(cache, assets, mode, id));
            let (model, scale) = match cube {
                Some(model) => (model, DROPPED_BLOCK_SCALE),
                None => {
                    let Some(model) =
                        Self::icon_model(cache, icons, identifier, view.item.identity.metadata)
                    else {
                        continue;
                    };
                    (model, SPRITE_WORLD_SCALE)
                }
            };
            let (block_level, sky_level) = stream.light_level_at(view.position);
            for offset in view.copy_offsets.iter().take(usize::from(view.copy_count)) {
                let center = std::array::from_fn(|axis| view.position[axis] + offset[axis]);
                instances.push(DroppedItemInstance {
                    model,
                    world_from_item: dropped_item_transform(center, view.yaw_radians, scale),
                    block_level: u32::from(block_level),
                    sky_level: u32::from(sky_level),
                    overlay_rgba8: 0,
                });
            }
        }

        if let Some(assets) = assets {
            for view in stream.block_entities(partial_tick) {
                let (id_mode, id, base_scale) = match &view.kind {
                    BlockEntityKind::Falling { block_runtime_id } => {
                        let Ok(id) = u32::try_from(*block_runtime_id) else {
                            continue;
                        };
                        (mode, id, FALLING_BLOCK_SCALE)
                    }
                    BlockEntityKind::PrimedTnt { visual } => match visual {
                        ItemVisualRoute::BlockItem(id) => (NetworkIdMode::Sequential, id.0, 1.0),
                        _ => continue,
                    },
                };
                let Some(model) = Self::block_model(cache, assets, id_mode, id) else {
                    continue;
                };
                let (block_level, sky_level) = stream.light_level_at(view.center);
                instances.push(DroppedItemInstance {
                    model,
                    world_from_item: dropped_item_transform(
                        view.center,
                        0.0,
                        base_scale * view.scale,
                    ),
                    block_level: u32::from(block_level),
                    sky_level: u32::from(sky_level),
                    overlay_rgba8: if view.flash {
                        pack_overlay_rgba8(TNT_FLASH_OVERLAY)
                    } else {
                        0
                    },
                });
            }
        }

        let mut lines: Vec<ItemMeshVertex> = Vec::new();
        if let Some(camera) = camera {
            for rope in stream.ropes(partial_tick) {
                let length = rope
                    .from
                    .iter()
                    .zip(&rope.to)
                    .map(|(a, b)| (a - b) * (a - b))
                    .sum::<f32>()
                    .sqrt();
                let (segments, sag, half_width, color) = match rope.kind {
                    RopeKind::FishingLine => (
                        FISHING_SEGMENTS,
                        length * FISHING_SAG_FRACTION,
                        FISHING_HALF_WIDTH,
                        rope_color(0, 0, 0),
                    ),
                    RopeKind::Lead => (
                        LEAD_SEGMENTS,
                        length * LEAD_SAG_FRACTION,
                        LEAD_HALF_WIDTH,
                        rope_color(127, 96, 55),
                    ),
                };
                rope_ribbon(
                    rope.from, rope.to, camera, segments, sag, half_width, color, &mut lines,
                );
            }
        }
        scene.publish(
            cache.revision,
            Arc::clone(&cache.shared),
            &instances,
            &lines,
            DAYLIGHT,
        );
    }
}
