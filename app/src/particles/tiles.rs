use std::sync::Arc;

use assets::{
    BlockFace, MATERIAL_FLAG_BIRCH_FOLIAGE, MATERIAL_FLAG_DRY_FOLIAGE,
    MATERIAL_FLAG_EVERGREEN_FOLIAGE, MATERIAL_FLAG_FOLIAGE_CLASS_MASK, MATERIAL_FLAG_FOLIAGE_TINT,
    MATERIAL_FLAG_GRASS_TINT, MATERIAL_FLAG_TINT_MASK, MATERIAL_FLAG_WATER_TINT, NetworkIdMode,
    RuntimeAssets, RuntimeIconCatalog,
};
use client_world::WorldStream;
use render::TileRequest;

/// Marks item-icon tile keys so they never collide with block layer keys.
const ITEM_KEY_FLAG: u64 = 1 << 63;

/// A block-textured particle tile with its gamma-space biome tint.
pub(super) struct BlockTile {
    pub(super) tile: TileRequest,
    pub(super) tint: [f32; 4],
}

fn linear_to_srgb(c: f32) -> f32 {
    let c = c.clamp(0.0, 1.0);
    if c <= 0.003_130_8 {
        c * 12.92
    } else {
        1.055 * c.powf(1.0 / 2.4) - 0.055
    }
}

/// The block's particle tile: its tinted top face when the top face is biome-tinted and the
/// side is not (grass), otherwise the side face, with the biome colour for that tint mode.
pub(super) fn block_tile(
    stream: &WorldStream,
    mode: NetworkIdMode,
    network_id: u32,
    block: [i32; 3],
) -> Option<BlockTile> {
    let assets: &RuntimeAssets = stream.runtime_assets();
    if !assets.is_known(mode, network_id) {
        return None;
    }
    let resolved = assets.resolve(mode, network_id);
    let side = assets.material(resolved.face(BlockFace::North).material_id());
    let top = assets.material(resolved.face(BlockFace::Up).material_id());
    let material =
        if side.flags & MATERIAL_FLAG_TINT_MASK == 0 && top.flags & MATERIAL_FLAG_TINT_MASK != 0 {
            top
        } else {
            side
        };
    let page = assets
        .texture_pages()
        .get(material.texture.page() as usize)?;
    let mip = page.texture.mips.first()?;
    let layer = material.texture.layer();
    if layer >= page.texture.layers {
        return None;
    }
    let stride = (mip.size * mip.size * 4) as usize;
    let start = layer as usize * stride;
    let pixels = mip.rgba8.get(start..start + stride)?;
    Some(BlockTile {
        tile: TileRequest {
            key: (u64::from(material.texture.page()) << 32) | u64::from(layer),
            size: mip.size,
            pixels: Arc::from(pixels),
        },
        tint: biome_tint(stream, material.flags, block),
    })
}

/// Gamma-space biome colour for a material's tint mode; white when untinted.
fn biome_tint(stream: &WorldStream, flags: u32, block: [i32; 3]) -> [f32; 4] {
    let mode = flags & MATERIAL_FLAG_TINT_MASK;
    if mode == 0 {
        return [1.0; 4];
    }
    let center = block.map(|c| c as f32 + 0.5);
    let Some(raw) = stream.camera_biome_id(center) else {
        return [1.0; 4];
    };
    let tints = stream.resolved_biome_tints_snapshot();
    let Some(record) = tints.records.get(tints.dense_index(raw) as usize) else {
        return [1.0; 4];
    };
    let linear = if mode == MATERIAL_FLAG_WATER_TINT {
        record.water
    } else if mode == MATERIAL_FLAG_GRASS_TINT {
        record.grass
    } else if mode == MATERIAL_FLAG_FOLIAGE_TINT {
        match flags & MATERIAL_FLAG_FOLIAGE_CLASS_MASK {
            MATERIAL_FLAG_BIRCH_FOLIAGE => record.birch,
            MATERIAL_FLAG_EVERGREEN_FOLIAGE => record.evergreen,
            MATERIAL_FLAG_DRY_FOLIAGE => record.dry_foliage,
            _ => record.foliage,
        }
    } else {
        return [1.0; 4];
    };
    [
        linear_to_srgb(linear[0]),
        linear_to_srgb(linear[1]),
        linear_to_srgb(linear[2]),
        1.0,
    ]
}

/// An item icon as a particle tile, keyed by its catalog sprite.
pub(super) fn item_tile(
    icons: &RuntimeIconCatalog,
    identifier: &str,
    metadata: u32,
) -> Option<TileRequest> {
    let index = icons.lookup_index(identifier, metadata)?;
    let sprite = icons.sprites().get(index)?;
    // The particle tile is square; a non-square sprite is skipped rather than distorted.
    if sprite.width != sprite.height || sprite.width == 0 {
        return None;
    }
    Some(TileRequest {
        key: ITEM_KEY_FLAG | index as u64,
        size: u32::from(sprite.width),
        pixels: Arc::clone(&sprite.rgba8),
    })
}
