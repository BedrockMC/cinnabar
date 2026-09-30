//! Plain opaque block cubes held in hand: six 16-texel tiles composed into one 48x32 sheet.

use std::collections::BTreeMap;

use assets::{
    BlockFace, BlockFlags, DIAGNOSTIC_MATERIAL, IconSprite, ItemVisualDefinitionRoute,
    NO_ANIMATION, NetworkIdMode, RuntimeAssets, RuntimeEntityAssets, VisualKind, VisualSupport,
};

pub(super) const TILE: usize = 16;
pub(super) const SHEET_WIDTH: usize = TILE * 3;
pub(super) const SHEET_HEIGHT: usize = TILE * 2;

pub(super) struct BlockSheets {
    pub(super) sheets: Vec<IconSprite>,
    /// Block visual id to its sheet index.
    pub(super) by_visual: BTreeMap<u32, usize>,
}

/// A sheet per distinct set of face tiles, for every block item whose block is an ordinary
/// opaque cube; other blocks are skipped.
pub(super) fn collect(world: &RuntimeAssets, entities: &RuntimeEntityAssets) -> BlockSheets {
    let mut sheets = Vec::new();
    let mut by_materials = BTreeMap::<[u32; 6], usize>::new();
    let mut by_visual = BTreeMap::new();
    if !world.provenance().is_complete() {
        return BlockSheets { sheets, by_visual };
    }
    for definition in entities.item_visuals() {
        let ItemVisualDefinitionRoute::BlockItem { block_visual } = definition.route else {
            continue;
        };
        let visual = block_visual.0;
        if by_visual.contains_key(&visual) || visual as usize >= world.visual_count() {
            continue;
        }
        let Some(materials) = cube_materials(world, visual) else {
            continue;
        };
        let index = match by_materials.get(&materials) {
            Some(index) => *index,
            None => {
                let Some(sheet) = compose_sheet(world, &materials) else {
                    continue;
                };
                sheets.push(sheet);
                by_materials.insert(materials, sheets.len() - 1);
                sheets.len() - 1
            }
        };
        by_visual.insert(visual, index);
    }
    BlockSheets { sheets, by_visual }
}

fn cube_materials(world: &RuntimeAssets, visual: u32) -> Option<[u32; 6]> {
    let block = world.resolve(NetworkIdMode::Sequential, visual);
    if !block.is_known()
        || block.kind() != VisualKind::Cube
        || block.support() != VisualSupport::Exact
        || block.flags() != (BlockFlags::CUBE_GEOMETRY | BlockFlags::OCCLUDES_FULL_FACE)
        || block.model_template().is_some()
        || block.animation().is_some()
    {
        return None;
    }
    let mut materials = [0; 6];
    for (slot, face) in materials.iter_mut().zip(BlockFace::ALL) {
        *slot = block.face(face).material_id();
        if *slot == DIAGNOSTIC_MATERIAL {
            return None;
        }
    }
    Some(materials)
}

fn compose_sheet(world: &RuntimeAssets, materials: &[u32; 6]) -> Option<IconSprite> {
    let mut rgba8 = vec![0u8; SHEET_WIDTH * SHEET_HEIGHT * 4];
    for (face, id) in materials.iter().enumerate() {
        let material = world.materials().get(*id as usize)?;
        if material.flags != 0 || material.animation != NO_ANIMATION {
            return None;
        }
        let page = world
            .texture_pages()
            .get(material.texture.page() as usize)?;
        let mip = page.texture.mips.first()?;
        if mip.size as usize != TILE || material.texture.layer() >= page.texture.layers {
            return None;
        }
        let tile_bytes = TILE * TILE * 4;
        let first = (material.texture.layer() as usize).checked_mul(tile_bytes)?;
        let tile = mip.rgba8.get(first..first.checked_add(tile_bytes)?)?;
        let (x, y) = ((face % 3) * TILE, (face / 3) * TILE);
        for row in 0..TILE {
            let target = ((y + row) * SHEET_WIDTH + x) * 4;
            rgba8[target..target + TILE * 4]
                .copy_from_slice(&tile[row * TILE * 4..(row + 1) * TILE * 4]);
        }
    }
    Some(IconSprite {
        width: SHEET_WIDTH as u16,
        height: SHEET_HEIGHT as u16,
        rgba8: rgba8.into(),
    })
}

/// The `[u0, v0, u1, v1]` region of each face's tile within a sheet placed at `region`.
pub(super) fn face_rects(region: [f32; 4]) -> [[f32; 4]; 6] {
    let (width, height) = (region[2] - region[0], region[3] - region[1]);
    std::array::from_fn(|face| {
        let (column, row) = ((face % 3) as f32, (face / 3) as f32);
        [
            region[0] + width * column / 3.0,
            region[1] + height * row / 2.0,
            region[0] + width * (column + 1.0) / 3.0,
            region[1] + height * (row + 1.0) / 2.0,
        ]
    })
}
