use crate::{
    Aabb, BlockPhysicsFlags, BlockPhysicsSample, CollisionQuery, CollisionWorld, WorldQueryError,
};

/// How far below a scaffolding top the feet may sit and still stand on it.
const TOP_TOLERANCE: f32 = 1.0e-6;

/// Scaffolding is solid only under the feet of a player who is not descending.
pub(super) struct ScaffoldingView<'a, W> {
    inner: &'a W,
    player: Aabb,
    descending: bool,
}

impl<'a, W: CollisionWorld> ScaffoldingView<'a, W> {
    /// Retains the pre-move box used by the native contextual collision query.
    pub(super) const fn new(inner: &'a W, player: Aabb, descending: bool) -> Self {
        Self {
            inner,
            player,
            descending,
        }
    }

    /// Identifies scaffold provenance without treating nearby solids as scaffolding.
    fn is_scaffolding(&self, block: [i32; 3]) -> Result<bool, WorldQueryError> {
        Ok(self
            .inner
            .block_physics(block)?
            .layers
            .iter()
            .any(|facts| facts.flags.contains(BlockPhysicsFlags::SCAFFOLDING)))
    }
}

impl<W: CollisionWorld> CollisionWorld for ScaffoldingView<'_, W> {
    fn collision_boxes(&self, query: Aabb) -> Result<CollisionQuery<Vec<Aabb>>, WorldQueryError> {
        let colliders = self.inner.collision_boxes_with_provenance(query)?;
        let mut kept = Vec::with_capacity(colliders.value.len());
        for collider in colliders.value {
            let bounds = collider.aabb;
            let solid_top = !self.descending
                && self.player.min.y as f32 >= bounds.max.y as f32 - TOP_TOLERANCE
                && self.player.max.x > bounds.min.x
                && self.player.min.x < bounds.max.x
                && self.player.max.z > bounds.min.z
                && self.player.min.z < bounds.max.z;
            if !solid_top
                && let Some(block) = collider.block
                && self.is_scaffolding(block)?
            {
                continue;
            }
            kept.push(collider.aabb);
        }
        Ok(CollisionQuery {
            value: kept,
            identity: colliders.identity,
        })
    }

    fn block_physics(&self, block: [i32; 3]) -> Result<BlockPhysicsSample, WorldQueryError> {
        self.inner.block_physics(block)
    }
}
