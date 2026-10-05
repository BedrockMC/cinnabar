mod grid;
mod visible_set;

use std::collections::HashSet;

use hashbrown::HashMap;

use meshing::Face;
use world::SubChunkKey;

pub(crate) use grid::ConnectivityGrid;
pub use visible_set::CaveVisibleSet;

use grid::Slot;

/// Entry marker for the camera node, whose exits are its own touched faces.
const CAMERA_ENTRY: u8 = 6;
const FACE_MASK: u64 = 0x3f;

/// Conservative face-connectivity BFS used before Bevy's per-entity frustum culling.
#[must_use]
pub(crate) fn cave_visible_sub_chunks(
    camera: SubChunkKey,
    connectivity: &ConnectivityGrid,
) -> HashSet<SubChunkKey> {
    let mut visible = CaveVisibleSet::default();
    fill_visible(
        camera,
        connectivity,
        &mut CaveVisibilityScratch::default(),
        &mut visible,
    );
    visible.iter().collect()
}

/// Reusable traversal storage; `visited` is all-zero between calls.
#[derive(Default)]
pub struct CaveVisibilityScratch {
    visited: Vec<u8>,
    touched: Vec<u32>,
    stack: Vec<(u32, u8)>,
    overflow_ids: HashMap<SubChunkKey, u32>,
    overflow_nodes: Vec<(SubChunkKey, u64)>,
    overflow_visited: Vec<u8>,
}

impl CaveVisibilityScratch {
    /// Dense id for `key`: its grid cell, or a slot past the cells for an overflow key.
    fn node(&mut self, grid: &ConnectivityGrid, key: SubChunkKey) -> Option<u32> {
        match grid.slot(key) {
            Slot::Cell(index, _) => Some(index),
            Slot::Overflow(value) => {
                let cells = grid.cell_count() as u32;
                let next = cells + self.overflow_nodes.len() as u32;
                let id = *self.overflow_ids.entry(key).or_insert(next);
                if id == next {
                    self.overflow_nodes.push((key, value.bits()));
                    self.overflow_visited.push(0);
                }
                Some(id)
            }
            Slot::Missing => None,
        }
    }

    fn describe(&self, grid: &ConnectivityGrid, node: u32) -> (SubChunkKey, u64) {
        let cells = grid.cell_count() as u32;
        if node < cells {
            grid.cell(node)
        } else {
            self.overflow_nodes[(node - cells) as usize]
        }
    }

    /// Records entry `bit`; true when this node and entry face were not yet visited.
    fn mark(&mut self, cells: u32, node: u32, bit: u8) -> bool {
        let mask = if node < cells {
            &mut self.visited[node as usize]
        } else {
            &mut self.overflow_visited[(node - cells) as usize]
        };
        if *mask & bit != 0 {
            return false;
        }
        if *mask == 0 {
            self.touched.push(node);
        }
        *mask |= bit;
        true
    }
}

/// Reuses traversal and output storage without changing portal or support-shell rules.
pub(crate) fn fill_visible(
    camera: SubChunkKey,
    grid: &ConnectivityGrid,
    scratch: &mut CaveVisibilityScratch,
    visible: &mut CaveVisibleSet,
) {
    visible.reset(camera, grid.dims());
    if scratch.visited.len() != grid.cell_count() {
        scratch.visited.clear();
        scratch.visited.resize(grid.cell_count(), 0);
    }
    scratch.overflow_ids.clear();
    scratch.overflow_nodes.clear();
    scratch.overflow_visited.clear();
    scratch.stack.clear();
    let Some(camera_node) = scratch.node(grid, camera) else {
        for key in grid.keys() {
            visible.insert(key);
        }
        return;
    };
    let cells = grid.cell_count() as u32;
    scratch.mark(cells, camera_node, 1 << CAMERA_ENTRY);
    scratch.stack.push((camera_node, CAMERA_ENTRY));
    // The reachable (node, entry face) states are order-independent, so a stack suffices.
    while let Some((node, entry)) = scratch.stack.pop() {
        let (key, bits) = scratch.describe(grid, node);
        let mut exits = if entry == CAMERA_ENTRY {
            touched_faces(bits)
        } else {
            (bits >> (u32::from(entry) * 6)) & FACE_MASK
        };
        while exits != 0 {
            let exit = Face::ALL[exits.trailing_zeros() as usize];
            exits &= exits - 1;
            let Some(next) = adjacent(key, exit) else {
                continue;
            };
            let Some(next_node) = scratch.node(grid, next) else {
                continue;
            };
            let entered = opposite(exit) as u8;
            if scratch.mark(cells, next_node, 1 << entered) {
                scratch.stack.push((next_node, entered));
            }
        }
    }
    // Visibility is per sub-chunk entity, not per connected air region. Once
    // any portal exposes an entity, models in another region of that entity
    // are drawn too. Keep exactly one loaded neighbour shell visible so those
    // models cannot float over support geometry hidden in an adjacent entity.
    // Snapshot first: newly added shell nodes must not recursively expand.
    for index in 0..scratch.touched.len() {
        let node = scratch.touched[index];
        let (key, _) = scratch.describe(grid, node);
        visible.insert(key);
        for face in Face::ALL {
            if let Some(neighbour) = adjacent(key, face)
                && grid.contains_key(&neighbour)
            {
                visible.insert(neighbour);
            }
        }
        if node < cells {
            scratch.visited[node as usize] = 0;
        }
    }
    scratch.touched.clear();
}

/// The camera node may leave through any face its own air touches: the matrix diagonal.
const fn touched_faces(bits: u64) -> u64 {
    let mut faces = 0;
    let mut face = 0;
    while face < 6 {
        faces |= ((bits >> (face * 7)) & 1) << face;
        face += 1;
    }
    faces
}

fn adjacent(key: SubChunkKey, face: Face) -> Option<SubChunkKey> {
    let (x, y, z) = match face {
        Face::NegativeX => (key.x.checked_sub(1)?, key.y, key.z),
        Face::PositiveX => (key.x.checked_add(1)?, key.y, key.z),
        Face::NegativeY => (key.x, key.y.checked_sub(1)?, key.z),
        Face::PositiveY => (key.x, key.y.checked_add(1)?, key.z),
        Face::NegativeZ => (key.x, key.y, key.z.checked_sub(1)?),
        Face::PositiveZ => (key.x, key.y, key.z.checked_add(1)?),
    };
    Some(SubChunkKey::new(key.dimension, x, y, z))
}

const fn opposite(face: Face) -> Face {
    match face {
        Face::NegativeX => Face::PositiveX,
        Face::PositiveX => Face::NegativeX,
        Face::NegativeY => Face::PositiveY,
        Face::PositiveY => Face::NegativeY,
        Face::NegativeZ => Face::PositiveZ,
        Face::PositiveZ => Face::NegativeZ,
    }
}

#[cfg(test)]
mod tests;
