//! The per-frame system, carrier loading and the caches behind them.

use std::{collections::HashMap, path::Path, sync::Arc};

use assets::{BlockEntityRouteKind, RuntimeBlockEntityAssets, RuntimeFontCatalog};
use bevy::prelude::*;
use render::{
    AtlasRect, BlockEntityFrame, BlockEntityKind, BlockEntityScene, BlockEntitySubmission,
    SceneClock, SignFace, SignModel,
};
use ui::TextLayoutCache;
use world::{BlockEntityKey, BlockEntityNbt, ChunkKey};

use super::{
    containers::{ContainerKind, ContainerLids, cue_is_open},
    cracks::CrackClock,
    describe::{Template, describe},
    sign_text,
    state::BlockState,
};
use crate::{
    local_player::LocalViewPose, movement::PhysicsCollisionRegistries, runtime::world::ClientWorld,
    ui_runtime::UiRuntime,
};

const BLOCK_ENTITY_ASSETS_FILENAME: &str = "vanilla-v1.mcbeben";
/// Block entities farther than this from the eye are not drawn.
const SCAN_RADIUS_BLOCKS: f32 = 64.0;
const MAX_SUBMISSIONS: usize = 4_096;
const TICKS_PER_SECOND: f64 = 20.0;
const TEXT_CACHE_ENTRIES: usize = 256;
const TEXT_CACHE_BYTES: usize = 2 * 1024 * 1024;

/// Reads the optional block-entity carrier next to the world carrier; on absence or
/// corruption logs once and returns a scene that draws nothing.
pub(crate) fn load_block_entity_scene(world_asset_path: &Path) -> BlockEntityScene {
    let path = world_asset_path.with_file_name(BLOCK_ENTITY_ASSETS_FILENAME);
    let mut scene = BlockEntityScene::default();
    let bytes = match std::fs::read(&path) {
        Ok(bytes) => bytes,
        Err(error) => {
            eprintln!(
                "block-entity carrier {} unavailable ({error}); block-entity models, sign text and break cracks are not drawn; rebuild with: make block-entity-assets",
                path.display()
            );
            return scene;
        }
    };
    match RuntimeBlockEntityAssets::decode(&bytes) {
        Ok(assets) => {
            eprintln!(
                "loaded block-entity carrier from {} ({} textures, atlas {:?})",
                path.display(),
                assets.placements().len(),
                assets.atlas_size()
            );
            scene.install_assets(&assets);
        }
        Err(error) => eprintln!(
            "block-entity carrier {} is invalid ({error}); block-entity models, sign text and break cracks are not drawn; rebuild with: make block-entity-assets",
            path.display()
        ),
    }
    scene
}

/// The UI font used to rasterize sign text.
#[derive(Resource)]
pub(crate) struct BlockEntityFont(pub(crate) Arc<RuntimeFontCatalog>);

struct BlockInfo {
    name: Arc<str>,
    state: BlockState,
}

struct Described {
    nbt: Arc<BlockEntityNbt>,
    runtime_id: u32,
    template: Option<Template>,
}

#[derive(Resource)]
pub(crate) struct BlockEntityRuntime {
    cracks: CrackClock,
    lids: ContainerLids,
    described: HashMap<BlockEntityKey, Described>,
    blocks: HashMap<u32, Option<Arc<BlockInfo>>>,
    layouts: TextLayoutCache,
}

impl BlockEntityRuntime {
    pub(crate) fn new() -> Self {
        Self {
            cracks: CrackClock::default(),
            lids: ContainerLids::default(),
            described: HashMap::new(),
            blocks: HashMap::new(),
            layouts: TextLayoutCache::new(TEXT_CACHE_ENTRIES, TEXT_CACHE_BYTES),
        }
    }
}

pub(crate) fn configure(app: &mut App, font: Arc<RuntimeFontCatalog>) {
    app.insert_resource(BlockEntityFont(font))
        .insert_resource(BlockEntityRuntime::new())
        .add_systems(Update, update_block_entity_scene);
}

/// Light multiplier for retained block/sky light levels; needs native measurement.
fn light_factor(block: u8, sky: u8) -> f32 {
    let level = f32::from(block.max(sky).min(15)) / 15.0;
    level.powf(1.6).max(0.04)
}

fn block_info(
    runtime: &mut BlockEntityRuntime,
    collisions: &PhysicsCollisionRegistries,
    mode: assets::NetworkIdMode,
    runtime_id: u32,
) -> Option<Arc<BlockInfo>> {
    runtime
        .blocks
        .entry(runtime_id)
        .or_insert_with(|| {
            let name = collisions.block_identifier(mode, runtime_id)?;
            let state = collisions
                .block_canonical_state(mode, runtime_id)
                .map(BlockState::parse)
                .unwrap_or_default();
            Some(Arc::new(BlockInfo {
                name: Arc::from(name),
                state,
            }))
        })
        .clone()
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn update_block_entity_scene(
    client_world: Res<ClientWorld>,
    collisions: Res<PhysicsCollisionRegistries>,
    view: Res<LocalViewPose>,
    ui: Res<UiRuntime>,
    time: Res<Time<Real>>,
    font: Res<BlockEntityFont>,
    mut runtime: ResMut<BlockEntityRuntime>,
    mut scene: ResMut<BlockEntityScene>,
    mut frame: ResMut<BlockEntityFrame>,
) {
    if !scene.has_assets() {
        return;
    }
    let now_seconds = time.elapsed_secs_f64();
    let clock = SceneClock {
        ticks: now_seconds * TICKS_PER_SECOND,
    };
    let Some(stream) = client_world.stream.as_ref() else {
        runtime.described.clear();
        *frame = scene.update(clock, &[], &[]).clone();
        return;
    };
    let runtime = &mut *runtime;
    let dimension = stream.current_dimension();
    let store = stream.collision_store();
    let mode = stream.network_id_mode();
    let eye = view.eye_translation();
    let delta = time.delta_secs();

    let cracks = ui
        .block_crack_snapshot()
        .filter(|snapshot| snapshot.dimension == dimension)
        .map_or_else(Vec::new, |snapshot| {
            runtime.cracks.instances(&snapshot.entries, now_seconds)
        });

    let mut submissions: Vec<BlockEntitySubmission> = Vec::new();
    let mut seen: Vec<BlockEntityKey> = Vec::new();
    runtime.lids.begin();
    let chunk_range = |center: f32| {
        ((center - SCAN_RADIUS_BLOCKS) / 16.0).floor() as i32
            ..=((center + SCAN_RADIUS_BLOCKS) / 16.0).floor() as i32
    };
    'columns: for chunk_x in chunk_range(eye.x) {
        for chunk_z in chunk_range(eye.z) {
            let Some(chunk) = store.chunk(ChunkKey::new(dimension, chunk_x, chunk_z)) else {
                continue;
            };
            for (key, nbt) in chunk.block_entities() {
                if submissions.len() >= MAX_SUBMISSIONS {
                    break 'columns;
                }
                let [x, y, z] = key.position();
                let center = Vec3::new(x as f32 + 0.5, y as f32 + 0.5, z as f32 + 0.5);
                if center.distance_squared(eye) > SCAN_RADIUS_BLOCKS * SCAN_RADIUS_BLOCKS {
                    continue;
                }
                let Some(id) = nbt.id() else {
                    continue;
                };
                if !matches!(
                    assets::block_entity_route(id),
                    Some(BlockEntityRouteKind::Model | BlockEntityRouteKind::TextOverlay)
                ) {
                    continue;
                }
                let Some(runtime_id) = store.sub_chunk(key.sub_chunk()).and_then(|sub_chunk| {
                    sub_chunk.runtime_id(0, (x & 15) as u8, (y & 15) as u8, (z & 15) as u8)
                }) else {
                    continue;
                };
                seen.push(key);
                let stale = runtime.described.get(&key).is_none_or(|entry| {
                    !Arc::ptr_eq(&entry.nbt, &nbt) || entry.runtime_id != runtime_id
                });
                if stale {
                    let template = block_info(runtime, &collisions, mode, runtime_id)
                        .zip(nbt.parse())
                        .and_then(|(info, root)| {
                            describe(id, &info.name, &info.state, &root, [x, y, z])
                        });
                    runtime.described.insert(
                        key,
                        Described {
                            nbt: Arc::clone(&nbt),
                            runtime_id,
                            template,
                        },
                    );
                }
                let Some(template) = runtime
                    .described
                    .get(&key)
                    .and_then(|entry| entry.template.clone())
                else {
                    continue;
                };
                let (block_light, sky_light) = stream.light_level_at(center.to_array());
                let kind = resolve(
                    template,
                    [x, y, z],
                    eye,
                    delta,
                    stream,
                    runtime,
                    &mut scene,
                    &font.0,
                );
                if let Some(kind) = kind {
                    submissions.push(BlockEntitySubmission {
                        block: [x, y, z],
                        light: light_factor(block_light, sky_light),
                        kind,
                    });
                }
            }
        }
    }
    runtime.lids.finish();
    runtime.described.retain(|key, _| seen.contains(key));
    *frame = scene.update(clock, &cracks, &submissions).clone();
}

/// Applies per-frame state (lid openness, sign canvases, viewer yaw) to a template.
#[allow(clippy::too_many_arguments)]
fn resolve(
    template: Template,
    position: [i32; 3],
    eye: Vec3,
    delta_seconds: f32,
    stream: &client_world::WorldStream,
    runtime: &mut BlockEntityRuntime,
    scene: &mut BlockEntityScene,
    font: &RuntimeFontCatalog,
) -> Option<BlockEntityKind> {
    let open_at = |at: [i32; 3]| {
        stream
            .block_event_cue(at)
            .is_some_and(|cue| cue_is_open(cue.event_type, cue.event_value))
    };
    match template {
        Template::Static(kind) => Some(kind),
        Template::Chest(mut model) => {
            let open = open_at(position)
                || matches!(model.pair, render::ChestPair::Lead { partner } if open_at(partner));
            if !matches!(model.pair, render::ChestPair::Follower) {
                model.lid =
                    runtime
                        .lids
                        .advance(position, ContainerKind::Chest, open, delta_seconds);
            }
            Some(BlockEntityKind::Chest(model))
        }
        Template::Shulker(mut model) => {
            model.open = runtime.lids.advance(
                position,
                ContainerKind::Shulker,
                open_at(position),
                delta_seconds,
            );
            Some(BlockEntityKind::Shulker(model))
        }
        Template::EnchantTable => {
            let (dx, dz) = (
                eye.x - (position[0] as f32 + 0.5),
                eye.z - (position[2] as f32 + 0.5),
            );
            Some(BlockEntityKind::EnchantTable {
                facing_yaw_degrees: (-dx).atan2(-dz).to_degrees(),
            })
        }
        Template::Sign { mount, front, back } => {
            let mut face = |spec: Option<sign_text::SignTextSpec>| -> Option<SignFace> {
                let spec = spec?;
                let rect: AtlasRect = scene.text_rect(spec.cache_key(), || {
                    sign_text::rasterize(&spec, font, &mut runtime.layouts).unwrap_or_default()
                })?;
                Some(SignFace {
                    rect,
                    glowing: spec.glowing,
                })
            };
            let front = face(front);
            let back = face(back);
            (front.is_some() || back.is_some()).then_some(BlockEntityKind::Sign(SignModel {
                mount,
                front,
                back,
            }))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn light_maps_darkness_to_a_floor_and_full_light_to_one() {
        assert!((light_factor(15, 0) - 1.0).abs() < 1.0e-6);
        assert!((light_factor(0, 15) - 1.0).abs() < 1.0e-6);
        assert!((light_factor(0, 0) - 0.04).abs() < 1.0e-6);
        assert!(light_factor(8, 0) > light_factor(4, 0));
    }
}
