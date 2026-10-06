//! Camera collision scans borrow palette layers and keep lenient skip counts.

use super::{
    Aabb, BlockPhysics, LenientSkipCounts, PaletteWorld, SubChunkKey, Vec3, WorldQueryError,
    block_ceil, block_floor, validate_collision_query,
};

impl PaletteWorld<'_> {
    /// Emits known solids in cell order without constructing identities or shape vectors.
    pub(super) fn visit_camera_colliders(
        &self,
        query: Aabb,
        visitor: &mut dyn FnMut(Aabb),
    ) -> Result<LenientSkipCounts, WorldQueryError> {
        validate_collision_query(query)?;
        let mut skipped = LenientSkipCounts::default();
        if query.min == query.max {
            return Ok(skipped);
        }
        let grown = query.grown(1.0);
        let min = block_floor(grown.min)?;
        let max = block_ceil(grown.max)?;
        for x in min[0]..=max[0] {
            for z in min[2]..=max[2] {
                for y in min[1]..=max[1] {
                    let block = [x, y, z];
                    let key = SubChunkKey::new(self.dimension, x >> 4, y >> 4, z >> 4);
                    if !self.store.is_sub_chunk_loaded(key) {
                        skipped.unloaded_chunk = skipped.unloaded_chunk.saturating_add(1);
                        continue;
                    }
                    let chunk = self.store.sub_chunk(key);
                    let layers = chunk
                        .as_ref()
                        .map_or(1, |chunk| chunk.storages().len().max(1));
                    for layer in 0..layers {
                        let id = chunk
                            .as_ref()
                            .and_then(|chunk| {
                                chunk.runtime_id(
                                    layer,
                                    x.rem_euclid(16) as u8,
                                    y.rem_euclid(16) as u8,
                                    z.rem_euclid(16) as u8,
                                )
                            })
                            .unwrap_or(self.registry.air_runtime_id);
                        let Some(physics) = self.registry.physics(id) else {
                            skipped.unknown_runtime_id =
                                skipped.unknown_runtime_id.saturating_add(1);
                            continue;
                        };
                        match self.visit_camera_cell(block, physics, query, visitor) {
                            Ok(()) => {}
                            Err(WorldQueryError::UnloadedChunk(_)) => {
                                skipped.unloaded_chunk = skipped.unloaded_chunk.saturating_add(1);
                            }
                            Err(error) => return Err(error),
                        }
                    }
                }
            }
        }
        Ok(skipped)
    }

    /// Paired doors use one stack shape; other blocks borrow the registered shape slice.
    fn visit_camera_cell(
        &self,
        block: [i32; 3],
        physics: &BlockPhysics,
        query: Aabb,
        visitor: &mut dyn FnMut(Aabb),
    ) -> Result<(), WorldQueryError> {
        let offset = Vec3::new(block[0] as f64, block[1] as f64, block[2] as f64);
        if physics.door.is_some()
            && !Aabb::new(Vec3::ZERO, Vec3::ONE)
                .translated(offset)
                .intersects(query)
        {
            return Ok(());
        }
        let door = self.resolved_door_shape(block, physics)?;
        let shapes = door.as_slice();
        for shape in if door.is_some() {
            shapes
        } else {
            &physics.shapes
        } {
            let shape = shape.translated(offset);
            if shape.intersects(query) {
                visitor(shape);
            }
        }
        Ok(())
    }
}
