use client_world::WorldStream;
use render::{ParticleFluid, ParticleWorld};
use sim::{Aabb, BlockPhysicsFlags, CollisionRegistry, CollisionWorld, PaletteWorld, Vec3};

/// Live-session world access for particle collision, lighting and fluid checks.
pub(super) struct StreamParticleWorld<'a> {
    stream: &'a WorldStream,
    registry: &'a CollisionRegistry,
}

impl<'a> StreamParticleWorld<'a> {
    pub(super) fn new(stream: &'a WorldStream, registry: &'a CollisionRegistry) -> Self {
        Self { stream, registry }
    }

    fn palette(&self) -> PaletteWorld<'a> {
        PaletteWorld::new(
            self.stream.collision_store(),
            self.registry,
            self.stream.current_dimension(),
        )
    }

    /// The network runtime id at a block cell, when that chunk is loaded.
    pub(super) fn block_runtime_id(&self, block: [i32; 3]) -> Option<u32> {
        self.palette().primary_runtime_id(block).ok()
    }
}

impl ParticleWorld for StreamParticleWorld<'_> {
    fn solid_boxes(&self, min: [f32; 3], max: [f32; 3], out: &mut Vec<[f32; 6]>) {
        let query = Aabb::new(
            Vec3::new(f64::from(min[0]), f64::from(min[1]), f64::from(min[2])),
            Vec3::new(f64::from(max[0]), f64::from(max[1]), f64::from(max[2])),
        );
        if let Ok(found) = self.palette().collision_boxes_camera_lenient(query) {
            out.extend(found.value.iter().map(|shape| {
                [
                    shape.min.x as f32,
                    shape.min.y as f32,
                    shape.min.z as f32,
                    shape.max.x as f32,
                    shape.max.y as f32,
                    shape.max.z as f32,
                ]
            }));
        }
    }

    fn light(&self, block: [i32; 3]) -> (u8, u8) {
        self.stream.light_level_at(block.map(|c| c as f32 + 0.5))
    }

    fn fluid(&self, block: [i32; 3]) -> ParticleFluid {
        match self.palette().block_physics(block) {
            Ok(sample) => {
                let flags = sample.primary().flags;
                if flags.contains(BlockPhysicsFlags::WATER) {
                    ParticleFluid::Water
                } else if flags.contains(BlockPhysicsFlags::LAVA) {
                    ParticleFluid::Lava
                } else {
                    ParticleFluid::None
                }
            }
            Err(_) => ParticleFluid::None,
        }
    }
}
