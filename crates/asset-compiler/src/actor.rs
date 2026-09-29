//! Unconditional actor artwork for the explicitly incomplete neutral material profile.
use std::{collections::BTreeMap, io::Cursor, path::Path, sync::Arc};

use assets::{
    ActorArtworkBinding, ActorTexture, AssetError, CompiledEntityAssets, MAX_ACTOR_PIXEL_BYTES,
    MAX_ACTOR_TEXTURE_SIDE, MAX_ACTOR_TEXTURES, encode_actor_catalog, encode_entity_blob,
    neutral_actor_geometry_uvs_are_supported,
};
use image::{ImageFormat, ImageReader, Limits};
use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::entity::{compile_entity_assets, read_bounded_source};
mod pack;
pub use pack::{ActorPackCompilation, compile_actor_pack};

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ActorFallback {
    pub rig: u32,
    pub reason: Box<str>,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ActorCompileReport {
    pub source_manifest_sha256: [u8; 32],
    pub entity_carrier_sha256: [u8; 32],
    pub carrier_sha256: [u8; 32],
    pub textures: usize,
    pub bindings: usize,
    pub rest_pose_bindings: usize,
    pub pixel_bytes: usize,
    pub texture_evidence: Vec<ActorTextureEvidence>,
    pub fallbacks: Vec<ActorFallback>,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ActorTextureEvidence {
    pub source_path: Box<str>,
    pub source_sha256: [u8; 32],
    pub width: u16,
    pub height: u16,
    pub pixel_sha256: [u8; 32],
}
pub struct CompiledActorCarrier {
    pub bytes: Vec<u8>,
    pub report: ActorCompileReport,
}

pub fn compile_actor_assets(
    root: &Path,
    manifest: &[u8],
) -> Result<CompiledActorCarrier, AssetError> {
    let entities = compile_entity_assets(root, manifest)?;
    let entity_bytes = encode_entity_blob(&entities)?;
    let runtime_entities = assets::RuntimeEntityAssets::decode(&entity_bytes)?;
    let mut read = |index: u32| -> Result<Vec<u8>, AssetError> {
        let source = &entities.sources[index as usize];
        let bytes = read_bounded_source(root, &root.join(source.path.as_ref()))?;
        if bytes.len() != source.source_bytes as usize
            || <[u8; 32]>::from(Sha256::digest(&bytes)) != source.source_sha256
        {
            return Err(invalid("actor source changed after entity compilation"));
        }
        Ok(bytes)
    };
    let ArtworkBuild {
        textures,
        bindings,
        fallbacks,
        pixel_bytes,
    } = build_artwork(&entities, &runtime_entities, &mut read)?;
    let bytes = encode_actor_catalog(&entity_bytes, &textures, &bindings)?;
    let report = ActorCompileReport {
        source_manifest_sha256: entities.source_manifest_sha256,
        entity_carrier_sha256: Sha256::digest(&entity_bytes).into(),
        carrier_sha256: Sha256::digest(&bytes).into(),
        textures: textures.len(),
        bindings: bindings.len(),
        rest_pose_bindings: bindings
            .iter()
            .filter(|binding| binding.pose_mode == assets::ActorPoseMode::RestPose)
            .count(),
        pixel_bytes,
        texture_evidence: textures
            .iter()
            .map(|texture| {
                let source = &entities.sources[texture.source as usize];
                ActorTextureEvidence {
                    source_path: source.path.clone(),
                    source_sha256: source.source_sha256,
                    width: texture.width,
                    height: texture.height,
                    pixel_sha256: texture.pixel_sha256,
                }
            })
            .collect(),
        fallbacks,
    };
    Ok(CompiledActorCarrier { bytes, report })
}

/// Artwork for every rig with a decodable texture; `read` returns a source's bytes by index.
struct ArtworkBuild {
    textures: Vec<ActorTexture>,
    bindings: Vec<ActorArtworkBinding>,
    fallbacks: Vec<ActorFallback>,
    pixel_bytes: usize,
}

struct DecodedRaster {
    width: u16,
    height: u16,
    pixels: Vec<u8>,
}

fn decode_raster(path: &str, bytes: &[u8]) -> Option<DecodedRaster> {
    let format = if path.ends_with(".png") {
        ImageFormat::Png
    } else {
        ImageFormat::Tga
    };
    let mut reader = ImageReader::with_format(Cursor::new(bytes), format);
    let mut limits = Limits::default();
    limits.max_image_width = Some(MAX_ACTOR_TEXTURE_SIDE.into());
    limits.max_image_height = Some(MAX_ACTOR_TEXTURE_SIDE.into());
    limits.max_alloc = Some(4 * 1024 * 1024);
    reader.limits(limits);
    let image = reader.decode().ok()?;
    let (width, height) = (
        u16::try_from(image.width()).ok()?,
        u16::try_from(image.height()).ok()?,
    );
    let pixels = image.into_rgba8().into_raw();
    // The actor pipeline discards on alpha, so only binary alpha reproduces the raster.
    pixels
        .chunks_exact(4)
        .all(|pixel| matches!(pixel[3], 0 | 255))
        .then_some(DecodedRaster {
            width,
            height,
            pixels,
        })
}

fn build_artwork(
    entities: &CompiledEntityAssets,
    _runtime_entities: &assets::RuntimeEntityAssets,
    read: &mut dyn FnMut(u32) -> Result<Vec<u8>, AssetError>,
) -> Result<ArtworkBuild, AssetError> {
    let mut textures = Vec::<ActorTexture>::new();
    let mut bindings = Vec::new();
    let mut fallbacks = Vec::new();
    let mut pixel_bytes = 0usize;
    // Decoded once per source; `None` when the raster is unusable.
    let mut decoded = BTreeMap::<u32, Option<DecodedRaster>>::new();
    let mut table = BTreeMap::<u32, usize>::new();
    let render = &entities.render;
    for (rig_index, rig) in entities.rig_bindings.iter().enumerate() {
        let reject = |fallbacks: &mut Vec<ActorFallback>, reason: &str| {
            fallbacks.push(ActorFallback {
                rig: rig_index as u32,
                reason: reason.into(),
            });
        };
        let first = render
            .layers
            .partition_point(|layer| (layer.rig as usize) < rig_index);
        let layers = &render.layers[first..];
        let layers = &layers[..layers.partition_point(|layer| layer.rig as usize == rig_index)];
        let sources: Vec<u32> = layers
            .iter()
            .flat_map(|layer| {
                let slots = &render.slots[layer.first_slot as usize..][..layer.slot_count as usize];
                slots.iter().flat_map(|slot| {
                    render.candidates[slot.first_candidate as usize..]
                        [..slot.candidate_count as usize]
                        .iter()
                        .map(|candidate| candidate.source)
                })
            })
            .collect();
        if sources.is_empty() {
            reject(&mut fallbacks, "no_render_layer");
            continue;
        }
        for offset in 0..usize::from(rig.geometry_count) {
            let candidate_index = rig.first_geometry as usize + offset;
            let candidate = entities.rig_geometries[candidate_index];
            let geometry = &entities.geometries[candidate.geometry as usize];
            if !neutral_actor_geometry_uvs_are_supported(
                &entities.geometries,
                candidate.geometry as usize,
            ) {
                reject(&mut fallbacks, "uv_extents_or_inheritance");
                continue;
            }
            let mut default_texture = None;
            for &source in &sources {
                if !decoded.contains_key(&source) {
                    let path = entities.sources[source as usize].path.as_ref();
                    let raster = decode_raster(path, &read(source)?);
                    decoded.insert(source, raster);
                }
                let Some(raster) = decoded[&source].as_ref() else {
                    continue;
                };
                if raster.width != geometry.texture_width
                    || raster.height != geometry.texture_height
                {
                    continue;
                }
                let index = match table.get(&source) {
                    Some(&index) => index,
                    None => {
                        if textures.len() == MAX_ACTOR_TEXTURES
                            || pixel_bytes
                                .checked_add(raster.pixels.len())
                                .is_none_or(|total| total > MAX_ACTOR_PIXEL_BYTES)
                        {
                            reject(&mut fallbacks, "texture_budget");
                            continue;
                        }
                        pixel_bytes += raster.pixels.len();
                        textures.push(ActorTexture {
                            source,
                            width: raster.width,
                            height: raster.height,
                            pixel_sha256: Sha256::digest(&raster.pixels).into(),
                            rgba8: Arc::from(raster.pixels.as_slice()),
                        });
                        table.insert(source, textures.len() - 1);
                        textures.len() - 1
                    }
                };
                default_texture.get_or_insert(index);
            }
            let Some(texture) = default_texture else {
                reject(&mut fallbacks, "missing_or_ambiguous_texture");
                continue;
            };
            bindings.push(ActorArtworkBinding {
                rig: rig_index as u32,
                geometry_candidate: candidate_index as u32,
                entity_symbol: rig.entity_symbol,
                geometry: candidate.geometry,
                render_controller: rig.render_controller,
                texture: texture as u32,
                material: "entity".into(),
                pose_mode: assets::ActorPoseMode::CompiledLiteral,
            });
        }
    }
    Ok(ArtworkBuild {
        textures,
        bindings,
        fallbacks,
        pixel_bytes,
    })
}

fn invalid(detail: &str) -> AssetError {
    AssetError::InvalidCompiledAssets {
        detail: detail.into(),
    }
}
