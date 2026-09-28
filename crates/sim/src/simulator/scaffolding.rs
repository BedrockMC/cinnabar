use crate::{
    Aabb, BlockPhysicsFlags, BlockPhysicsSample, CollisionQuery, CollisionWorld, WorldQueryError,
};

/// How far below a scaffolding top the feet may sit and still stand on it.
const TOP_TOLERANCE: f64 = 1.0e-3;

/// Scaffolding is solid only under the feet of a player who is not descending.
pub(super) struct ScaffoldingView<'a, W> {
    inner: &'a W,
    feet_y: f64,
    descending: bool,
}

impl<'a, W: CollisionWorld> ScaffoldingView<'a, W> {
    pub(super) const fn new(inner: &'a W, feet_y: f64, descending: bool) -> Self {
        Self {
            inner,
            feet_y,
            descending,
        }
    }

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
            let solid_top = !self.descending && self.feet_y >= collider.aabb.max.y - TOP_TOLERANCE;
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
