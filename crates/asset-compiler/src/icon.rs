//! Item-icon compiler: bakes the exact sprite pixels for every sprite-routed
//! item visual (and alias) the entity compilation resolves from the pinned
//! pack, deduplicated by raster source, into the bounded icon carrier.

use std::{collections::BTreeMap, path::Path, sync::Arc};

use assets::{
    AssetError, IconEntry, IconSprite, ItemVisualDefinitionRoute, MAX_ICON_SIDE,
    encode_icon_catalog,
};
use sha2::{Digest, Sha256};

use crate::entity::compile_entity_assets;
use crate::image::decode_texture;

mod blocks;
mod cube;
mod model;

#[derive(Debug)]
pub struct CompiledIconCarrier {
    pub bytes: Vec<u8>,
    pub report: IconCompileReport,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IconCompileReport {
    pub source_manifest_sha256: [u8; 32],
    pub carrier_sha256: [u8; 32],
    pub sprites: usize,
    pub entries: usize,
    pub sprite_visuals: usize,
    pub alias_entries: usize,
    /// Vertical animation strips reduced to their first recorded frame.
    pub animation_strips: usize,
    /// Raster sources outside the flat-icon bounds, skipped and counted.
    pub skipped_oversized: usize,
    pub block_visuals: usize,
    /// Block items drawn as their flat carried texture.
    pub flat_block_visuals: usize,
    /// Non-cube 3D block items drawn from their isolated world template.
    pub model_block_visuals: usize,
    pub skipped_blocks: usize,
    /// Item identifiers of block items left without an icon.
    pub unresolved_block_items: Vec<Box<str>>,
    /// Geometry, material, texture, and alpha refusals in that order.
    pub block_refusals: [usize; 4],
    pub block_registry_sha256: Option<[u8; 32]>,
    pub block_policy: Option<&'static str>,
}

pub fn compile_icon_assets(
    root: &Path,
    source_manifest: &[u8],
) -> Result<CompiledIconCarrier, AssetError> {
    compile(root, source_manifest, None)
}

/// Adds bounded ordinary opaque-cube thumbnails after all original sprites.
/// Other block presentation remains unavailable; side shading is provisional.
pub fn compile_icon_assets_with_blocks(
    root: &Path,
    source_manifest: &[u8],
    world: &assets::RuntimeAssets,
) -> Result<CompiledIconCarrier, AssetError> {
    compile(root, source_manifest, Some(world))
}

fn compile(
    root: &Path,
    source_manifest: &[u8],
    world: Option<&assets::RuntimeAssets>,
) -> Result<CompiledIconCarrier, AssetError> {
    let compiled = compile_entity_assets(root, source_manifest)?;
    if let Some(world) = world {
        cube::validate_world(
            world,
            compiled.source_manifest_sha256,
            compiled.block_visual_count as usize,
        )?;
    }
    let icon_blocks = world.map(|_| blocks::IconBlocks::read(root)).transpose()?;
    let mut block_plan = BTreeMap::new();
    let mut flat_plan = BTreeMap::new();
    if let (Some(world), Some(flat)) = (world, icon_blocks.as_ref()) {
        for visual in compiled.item_visuals.iter() {
            let ItemVisualDefinitionRoute::BlockItem {
                block_visual: block,
            } = visual.route
            else {
                continue;
            };
            if flat.is_flat(world, block) {
                flat_plan
                    .entry(block.0)
                    .or_insert_with(|| flat.texture_path(block));
            } else if !block_plan.contains_key(&block.0) {
                if block_plan.len() == cube::MAX_BLOCK_ICONS {
                    return Err(cube::invalid("block icon route count exceeds 1024"));
                }
                block_plan.insert(block.0, cube::Cube::read(world, block));
            }
        }
    }
    let mut sprites: Vec<IconSprite> = Vec::new();
    let mut sprite_by_source: BTreeMap<u32, Option<u32>> = BTreeMap::new();
    let mut animation_strips = 0usize;
    let mut skipped_oversized = 0usize;
    let mut entries: Vec<IconEntry> = Vec::new();
    let mut sprite_visuals = 0usize;

    let mut sprite_for_source =
        |source_index: u32, sprites: &mut Vec<IconSprite>| -> Result<Option<u32>, AssetError> {
            if let Some(existing) = sprite_by_source.get(&source_index) {
                return Ok(*existing);
            }
            let source = &compiled.sources[source_index as usize];
            let decoded = decode_texture(&root.join(source.path.as_ref()), &source.path)?;
            let Some((sprite, strip)) = bounded_sprite(decoded) else {
                skipped_oversized += 1;
                sprite_by_source.insert(source_index, None);
                return Ok(None);
            };
            animation_strips += usize::from(strip);
            let index =
                u32::try_from(sprites.len()).map_err(|_| AssetError::InvalidCompiledAssets {
                    detail: "icon sprite count exceeds platform".into(),
                })?;
            sprites.push(sprite);
            sprite_by_source.insert(source_index, Some(index));
            Ok(Some(index))
        };

    let mut visual_sprites: Vec<Option<u32>> = Vec::with_capacity(compiled.item_visuals.len());
    for visual in compiled.item_visuals.iter() {
        let sprite = match visual.route {
            ItemVisualDefinitionRoute::Sprite { texture } => {
                sprite_visuals += 1;
                sprite_for_source(texture.source, &mut sprites)?
            }
            ItemVisualDefinitionRoute::BlockItem { .. }
            | ItemVisualDefinitionRoute::EmptyHand
            | ItemVisualDefinitionRoute::Missing => None,
        };
        if let Some(sprite) = sprite {
            entries.push(IconEntry {
                identifier: visual.key.identifier.clone(),
                metadata: visual.key.metadata,
                sprite,
            });
        }
        visual_sprites.push(sprite);
    }
    let mut flat_by_path: BTreeMap<Box<str>, Option<u32>> = BTreeMap::new();
    let mut flat_sprites = BTreeMap::new();
    for (&visual, path) in &flat_plan {
        let Some(path) = path else {
            continue;
        };
        let sprite = match flat_by_path.get(path) {
            Some(existing) => *existing,
            None => {
                let sprite = blocks::IconBlocks::sprite(root, path)?.map(|sprite| {
                    let existing = sprites.iter().position(|known| *known == sprite);
                    existing.unwrap_or_else(|| {
                        sprites.push(sprite);
                        sprites.len() - 1
                    }) as u32
                });
                flat_by_path.insert(path.clone(), sprite);
                sprite
            }
        };
        if let Some(sprite) = sprite {
            flat_sprites.insert(visual, sprite);
        }
    }
    let mut model_sprites = BTreeMap::new();
    if let Some(world) = world {
        for (&visual, plan) in &block_plan {
            if plan.is_ok() {
                continue;
            }
            let Some(raster) = model_raster(root, world, icon_blocks.as_ref(), visual)? else {
                continue;
            };
            let index = sprites
                .iter()
                .position(|known| *known == raster)
                .unwrap_or_else(|| {
                    sprites.push(raster);
                    sprites.len() - 1
                });
            model_sprites.insert(visual, index as u32);
        }
    }
    // The legacy sprite-only entry point retains its exact accepted-input
    // behavior: unsupported keys never occupied that carrier. Only the new
    // world-aware route needs the conservative full-catalog preflight.
    if world.is_some() {
        let mut predicted_bytes = 96usize;
        for sprite in &sprites {
            predicted_bytes = predicted_bytes
                .checked_add(4 + sprite.rgba8.len())
                .ok_or_else(|| cube::invalid("icon byte count overflow"))?;
        }
        let block_count = block_plan.values().filter(|value| value.is_ok()).count();
        if sprites.len() + block_count > assets::MAX_ICON_SPRITES {
            return Err(cube::invalid("merged icon sprite count exceeds bound"));
        }
        predicted_bytes = predicted_bytes
            .checked_add(block_count * (4 + cube::PIXEL_BYTES))
            .ok_or_else(|| cube::invalid("icon byte count overflow"))?;
        let mut predicted_entries = 0usize;
        for key in compiled
            .item_visuals
            .iter()
            .map(|visual| &visual.key)
            .chain(compiled.item_visual_aliases.iter().map(|alias| &alias.key))
        {
            if key.identifier.len() > assets::MAX_ICON_KEY_BYTES {
                return Err(cube::invalid("icon key exceeds bound"));
            }
            predicted_entries += 1;
            predicted_bytes = predicted_bytes
                .checked_add(10 + key.identifier.len())
                .ok_or_else(|| cube::invalid("icon byte count overflow"))?;
        }
        if predicted_entries > assets::MAX_ICON_ENTRIES
            || predicted_bytes > assets::MAX_ICON_CARRIER_BYTES
        {
            return Err(cube::invalid("merged icon carrier exceeds bound"));
        }
    }
    let mut baked: BTreeMap<[u8; 32], Vec<(&cube::Cube<'_>, u32)>> = BTreeMap::new();
    let mut output_hashes: BTreeMap<[u8; 32], Vec<u32>> = BTreeMap::new();
    for (index, sprite) in sprites.iter().enumerate() {
        output_hashes
            .entry(Sha256::digest(&sprite.rgba8).into())
            .or_default()
            .push(index as u32);
    }
    let mut block_sprites = BTreeMap::new();
    for (&visual, plan) in &block_plan {
        let Ok(plan) = plan else {
            continue;
        };
        let sprite = if let Some((_, index)) = baked
            .get(&plan.digest())
            .into_iter()
            .flat_map(|bucket| bucket.iter())
            .find(|(previous, _)| plan.same_source(previous))
        {
            *index
        } else {
            let raster = plan.raster();
            let output: [u8; 32] = Sha256::digest(&raster.rgba8).into();
            // Hash collisions still compare full pixels and dimensions.
            let index = output_hashes
                .get(&output)
                .into_iter()
                .flat_map(|bucket| bucket.iter())
                .find(|&&index| sprites[index as usize] == raster)
                .copied()
                .unwrap_or_else(|| {
                    let index = sprites.len() as u32;
                    sprites.push(raster);
                    output_hashes.entry(output).or_default().push(index);
                    index
                });
            baked.entry(plan.digest()).or_default().push((plan, index));
            index
        };
        block_sprites.insert(visual, sprite);
    }
    let mut block_visuals = 0usize;
    let mut flat_block_visuals = 0usize;
    let mut model_block_visuals = 0usize;
    let mut skipped_blocks = 0usize;
    let mut block_refusals = [0usize; 4];
    let mut unresolved_block_items = Vec::new();
    for (index, visual) in compiled.item_visuals.iter().enumerate() {
        if let ItemVisualDefinitionRoute::BlockItem {
            block_visual: block,
        } = visual.route
        {
            let flat_sprite = flat_sprites.get(&block.0);
            let model_sprite = model_sprites.get(&block.0);
            flat_block_visuals += usize::from(flat_sprite.is_some());
            model_block_visuals += usize::from(model_sprite.is_some());
            if let Some(&sprite) = flat_sprite
                .or(model_sprite)
                .or_else(|| block_sprites.get(&block.0))
            {
                block_visuals += 1;
                visual_sprites[index] = Some(sprite);
                entries.push(IconEntry {
                    identifier: visual.key.identifier.clone(),
                    metadata: visual.key.metadata,
                    sprite,
                });
            } else if world.is_some() {
                skipped_blocks += 1;
                unresolved_block_items.push(visual.key.identifier.clone());
                if let Some(Err(reason)) = block_plan.get(&block.0) {
                    block_refusals[*reason as usize] += 1;
                }
            }
        }
    }
    let mut alias_entries = 0usize;
    for alias in compiled.item_visual_aliases.iter() {
        if let Some(sprite) = visual_sprites[alias.visual.0 as usize] {
            alias_entries += 1;
            entries.push(IconEntry {
                identifier: alias.key.identifier.clone(),
                metadata: alias.key.metadata,
                sprite,
            });
        }
    }
    entries.sort_by(|a, b| {
        (a.identifier.as_ref(), a.metadata).cmp(&(b.identifier.as_ref(), b.metadata))
    });

    let bytes = encode_icon_catalog(compiled.source_manifest_sha256, &sprites, &entries)?;
    Ok(CompiledIconCarrier {
        report: IconCompileReport {
            source_manifest_sha256: compiled.source_manifest_sha256,
            carrier_sha256: Sha256::digest(&bytes).into(),
            sprites: sprites.len(),
            entries: entries.len(),
            sprite_visuals,
            alias_entries,
            animation_strips,
            skipped_oversized,
            block_visuals,
            flat_block_visuals,
            model_block_visuals,
            skipped_blocks,
            unresolved_block_items,
            block_refusals,
            block_registry_sha256: world.map(|world| world.provenance().block_registry_sha256),
            block_policy: world.map(|_| cube::POLICY),
        },
        bytes,
    })
}

/// Bounds a decoded texture to a flat icon: as-is within `MAX_ICON_SIDE`, else a vertical
/// animation strip's first frame (`true`); anything else is refused.
fn bounded_sprite(decoded: crate::image::DecodedTexture) -> Option<(IconSprite, bool)> {
    let (width, height, rgba8, strip) =
        if decoded.width <= MAX_ICON_SIDE && decoded.height <= MAX_ICON_SIDE {
            (decoded.width, decoded.height, decoded.rgba8, false)
        } else if decoded.width <= MAX_ICON_SIDE
            && decoded.height > decoded.width
            && decoded.height.is_multiple_of(decoded.width.max(1))
        {
            // A vertical animation strip (compass, clock): the flat inventory icon is the
            // strip's first recorded frame, never a guessed crop.
            let frame_bytes = decoded.width as usize * decoded.width as usize * 4;
            let frame = decoded.rgba8[..frame_bytes].to_vec().into_boxed_slice();
            (decoded.width, decoded.width, frame, true)
        } else {
            return None;
        };
    let sprite = IconSprite {
        width: u16::try_from(width).ok()?,
        height: u16::try_from(height).ok()?,
        rgba8: Arc::from(rgba8),
    };
    Some((sprite, strip))
}

/// A 3D inventory thumbnail of state `visual` of a session block overlay; `None` when it has no
/// drawable geometry or needs a biome tint.
#[must_use]
pub fn overlay_block_icon(overlay: &assets::BlockOverlay, visual: usize) -> Option<IconSprite> {
    model::Model::overlay(overlay, visual)
        .ok()
        .map(|model| model.raster())
}

/// A 3D thumbnail for a block item the opaque-cube path refused: the item's icon state from the
/// world carrier, else a full cube of the block's carried textures (leaves).
fn model_raster(
    root: &Path,
    world: &assets::RuntimeAssets,
    blocks: Option<&blocks::IconBlocks>,
    visual: u32,
) -> Result<Option<IconSprite>, AssetError> {
    let visual = assets::BlockVisualId(visual);
    let state = blocks.map_or(visual, |blocks| blocks.icon_state(visual));
    if let Ok(model) = model::Model::read(world, state) {
        return Ok(Some(model.raster()));
    }
    let Some(paths) = blocks.and_then(|blocks| blocks.carried_faces(visual)) else {
        return Ok(None);
    };
    let mut tiles = Vec::with_capacity(6);
    for path in &paths {
        let Some(tile) = blocks::IconBlocks::tile(root, path)? else {
            return Ok(None);
        };
        tiles.push(tile);
    }
    let tiles: [Box<[u8]>; 6] = tiles.try_into().expect("six faces");
    Ok(Some(model::Model::cube(tiles).raster()))
}
