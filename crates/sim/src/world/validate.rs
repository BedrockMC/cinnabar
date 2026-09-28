//! Registry-time checks on a block's collision shapes and physics facts.

use super::{Aabb, BlockPhysicsFlags, RegistryError, SurfaceResponse};

pub(super) fn validate_facts(
    runtime_id: u32,
    shapes: &[Aabb],
    fluid_height: f64,
    flags: BlockPhysicsFlags,
    response: SurfaceResponse,
) -> Result<(), RegistryError> {
    let water = flags.contains(BlockPhysicsFlags::WATER);
    let lava = flags.contains(BlockPhysicsFlags::LAVA);
    let fluid = water || lava;
    let bubble = matches!(
        response,
        SurfaceResponse::BubbleUp | SurfaceResponse::BubbleDown
    );
    if (water && lava)
        || ((fluid_height > 0.0) != fluid)
        || (bubble && !water)
        || (flags.contains(BlockPhysicsFlags::PASSABLE) && fluid && !shapes.is_empty())
    {
        return Err(RegistryError::ContradictoryFacts { runtime_id });
    }
    Ok(())
}

pub(super) fn validate_shapes(runtime_id: u32, shapes: &[Aabb]) -> Result<(), RegistryError> {
    for (shape_index, shape) in shapes.iter().enumerate() {
        let coordinates = [
            shape.min.x,
            shape.min.y,
            shape.min.z,
            shape.max.x,
            shape.max.y,
            shape.max.z,
        ];
        if !coordinates.into_iter().all(f64::is_finite)
            || shape.min.x > shape.max.x
            || shape.min.y > shape.max.y
            || shape.min.z > shape.max.z
        {
            return Err(RegistryError::InvalidShape {
                runtime_id,
                shape_index,
            });
        }
        if shape.min.x < -1.0
            || shape.min.y < -1.0
            || shape.min.z < -1.0
            || shape.max.x > 2.0
            || shape.max.y > 2.0
            || shape.max.z > 2.0
        {
            return Err(RegistryError::ShapeOutsideLocalHalo {
                runtime_id,
                shape_index,
            });
        }
    }
    Ok(())
}
