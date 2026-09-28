//! Unconditional actor artwork for the explicitly incomplete neutral material profile.
use std::{collections::BTreeMap, io::Cursor, path::Path, sync::Arc};

use assets::{
    ActorArtworkBinding, ActorTexture, AssetError, CompiledEntityAssets, MAX_ACTOR_PIXEL_BYTES,
    MAX_ACTOR_TEXTURE_SIDE, MAX_ACTOR_TEXTURES, encode_actor_catalog, encode_entity_blob,
    neutral_actor_geometry_uvs_are_supported, neutral_actor_material_is_supported,
};
use image::{ImageFormat, ImageReader, Limits};
use serde::Serialize;
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::entity::{compile_entity_assets, parse_fully_unique_json, read_bounded_source};
mod eligibility;
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

/// Neutral artwork for every eligible rig of a catalog; `read` returns a source's bytes by index.
struct ArtworkBuild {
    textures: Vec<ActorTexture>,
    bindings: Vec<ActorArtworkBinding>,
    fallbacks: Vec<ActorFallback>,
    pixel_bytes: usize,
}

fn build_artwork(
    entities: &CompiledEntityAssets,
    runtime_entities: &assets::RuntimeEntityAssets,
    read: &mut dyn FnMut(u32) -> Result<Vec<u8>, AssetError>,
) -> Result<ArtworkBuild, AssetError> {
    let mut json_cache = BTreeMap::<u32, Value>::new();
    let mut textures = Vec::<ActorTexture>::new();
    let mut bindings = Vec::new();
    let mut fallbacks = Vec::new();
    let mut pixel_bytes = 0usize;
    for (rig_index, rig) in entities.rig_bindings.iter().enumerate() {
        let rig_index = rig_index as u32;
        let reject = |fallbacks: &mut Vec<ActorFallback>, reason: &str| {
            fallbacks.push(ActorFallback {
                rig: rig_index,
                reason: reason.into(),
            });
        };
        if rig.geometry_count != 1 {
            reject(&mut fallbacks, "conditional_selection");
            continue;
        }
        let candidate = entities.rig_geometries[rig.first_geometry as usize];
        if candidate.condition.is_some() {
            reject(&mut fallbacks, "conditional_selection");
            continue;
        }
        let entity_symbol = &entities.symbols[rig.entity_symbol as usize];
        let controller_symbol = &entities.symbols[rig.render_controller as usize];
        for source_index in [entity_symbol.source_index, controller_symbol.source_index] {
            if let std::collections::btree_map::Entry::Vacant(entry) =
                json_cache.entry(source_index)
            {
                let source = &entities.sources[source_index as usize];
                let bytes = read(source_index)?;
                entry.insert(parse_fully_unique_json(
                    Path::new(source.path.as_ref()),
                    &bytes,
                )?);
            }
        }
        let entity_json = json_cache[&entity_symbol.source_index].clone();
        let controller_json = json_cache[&controller_symbol.source_index].clone();
        if !eligibility::supported(entities, rig_index as usize, &entity_json, |index| {
            if let Some(value) = json_cache.get(&index) {
                return Ok(value.clone());
            }
            let source = &entities.sources[index as usize];
            let bytes = read(index)?;
            let value = parse_fully_unique_json(Path::new(source.path.as_ref()), &bytes)?;
            json_cache.insert(index, value.clone());
            Ok(value)
        })? {
            reject(&mut fallbacks, "unsupported_authored_state");
            continue;
        }
        let Some(pose_mode) =
            assets::neutral_actor_pose_mode(runtime_entities, rig.first_geometry as usize)
        else {
            reject(&mut fallbacks, "unsupported_pose_program");
            continue;
        };
        let description = &entity_json["minecraft:client_entity"]["description"];
        let controller =
            &controller_json["render_controllers"][controller_symbol.identifier.as_ref()];
        let Some(route) = resolve_route(description, controller) else {
            reject(&mut fallbacks, "conditional_selection");
            continue;
        };
        if !neutral_actor_material_is_supported(route.material) {
            reject(&mut fallbacks, "unsupported_material");
            continue;
        }
        if controller.as_object().is_none_or(|object| {
            object
                .keys()
                .any(|key| !matches!(key.as_str(), "geometry" | "materials" | "textures"))
        }) {
            reject(&mut fallbacks, "unsupported_render_state");
            continue;
        }
        let geometry = &entities.geometries[candidate.geometry as usize];
        if geometry.identifier.as_ref() != route.geometry {
            reject(&mut fallbacks, "geometry_identity");
            continue;
        }
        if !neutral_actor_geometry_uvs_are_supported(
            &entities.geometries,
            candidate.geometry as usize,
        ) {
            reject(&mut fallbacks, "uv_extents_or_inheritance");
            continue;
        }
        let possible: Vec<_> = entities
            .sources
            .iter()
            .enumerate()
            .filter(|(_, source)| {
                source.path.as_ref() == format!("{}.png", route.texture)
                    || source.path.as_ref() == format!("{}.tga", route.texture)
            })
            .collect();
        if possible.len() != 1 || !route.texture.starts_with("textures/entity/") {
            reject(&mut fallbacks, "missing_or_ambiguous_texture");
            continue;
        }
        let (source_index, source) = possible[0];
        let bytes = read(source_index as u32)?;
        let format = if source.path.ends_with(".png") {
            ImageFormat::Png
        } else {
            ImageFormat::Tga
        };
        let mut reader = ImageReader::with_format(Cursor::new(&bytes), format);
        let mut limits = Limits::default();
        limits.max_image_width = Some(MAX_ACTOR_TEXTURE_SIDE.into());
        limits.max_image_height = Some(MAX_ACTOR_TEXTURE_SIDE.into());
        limits.max_alloc = Some(4 * 1024 * 1024);
        reader.limits(limits);
        let image = match reader.decode() {
            Ok(image) => image,
            Err(_) => {
                reject(&mut fallbacks, "raster_decode_or_bounds");
                continue;
            }
        };
        if image.width() != u32::from(geometry.texture_width)
            || image.height() != u32::from(geometry.texture_height)
        {
            reject(&mut fallbacks, "texture_dimensions");
            continue;
        }
        let pixels = image.into_rgba8().into_raw();
        if pixels
            .chunks_exact(4)
            .any(|pixel| !matches!(pixel[3], 0 | 255))
        {
            reject(&mut fallbacks, "fractional_alpha");
            continue;
        }
        let hash: [u8; 32] = Sha256::digest(&pixels).into();
        let texture = if let Some(index) = textures.iter().position(|texture| {
            texture.width == geometry.texture_width
                && texture.height == geometry.texture_height
                && texture.pixel_sha256 == hash
                && texture.rgba8.as_ref() == pixels.as_slice()
        }) {
            index
        } else {
            if textures.len() == MAX_ACTOR_TEXTURES
                || pixel_bytes
                    .checked_add(pixels.len())
                    .is_none_or(|total| total > MAX_ACTOR_PIXEL_BYTES)
            {
                reject(&mut fallbacks, "texture_budget");
                continue;
            }
            pixel_bytes += pixels.len();
            textures.push(ActorTexture {
                source: source_index as u32,
                width: geometry.texture_width,
                height: geometry.texture_height,
                pixel_sha256: hash,
                rgba8: Arc::from(pixels),
            });
            textures.len() - 1
        };
        bindings.push(ActorArtworkBinding {
            rig: rig_index,
            geometry_candidate: rig.first_geometry,
            entity_symbol: rig.entity_symbol,
            geometry: candidate.geometry,
            render_controller: rig.render_controller,
            texture: texture as u32,
            material: route.material.into(),
            pose_mode,
        });
        if pose_mode == assets::ActorPoseMode::RestPose {
            reject(&mut fallbacks, pose_mode.reason());
        }
    }
    Ok(ArtworkBuild {
        textures,
        bindings,
        fallbacks,
        pixel_bytes,
    })
}

struct Route<'a> {
    geometry: &'a str,
    texture: &'a str,
    material: &'a str,
}
fn resolve_route<'a>(description: &'a Value, controller: &'a Value) -> Option<Route<'a>> {
    let geometry = alias(
        description,
        "geometry",
        controller["geometry"].as_str()?,
        "Geometry.",
    )?;
    let materials = controller["materials"].as_array()?;
    let textures = controller["textures"].as_array()?;
    if materials.len() != 1 || textures.len() != 1 {
        return None;
    }
    let material = materials[0].as_object()?;
    if material.len() != 1 {
        return None;
    }
    Some(Route {
        geometry,
        texture: alias(description, "textures", textures[0].as_str()?, "Texture.")?,
        material: alias(
            description,
            "materials",
            material.get("*")?.as_str()?,
            "Material.",
        )?,
    })
}
fn alias<'a>(
    description: &'a Value,
    family: &str,
    expression: &str,
    prefix: &str,
) -> Option<&'a str> {
    let key = expression.strip_prefix(prefix)?;
    if key.is_empty()
        || !key
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
    {
        return None;
    }
    description[family].as_object()?.get(key)?.as_str()
}
fn invalid(detail: &str) -> AssetError {
    AssetError::InvalidCompiledAssets {
        detail: detail.into(),
    }
}
