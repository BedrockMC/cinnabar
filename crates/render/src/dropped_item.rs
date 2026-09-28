//! Dropped-item scene data: extruded sprite meshes drawn as world-space instances.
use bevy::{prelude::Resource, render::extract_resource::ExtractResource};
use std::sync::Arc;

mod mesh;

pub use mesh::{ITEM_MESH_VERTEX_BYTES, ItemMeshVertex, extruded_sprite_mesh};

/// Side length of every sprite layer on the GPU; larger sprites are rejected.
pub const MAX_ITEM_SPRITE_SIDE: u32 = 32;
pub const MAX_ITEM_SPRITES: usize = 512;
pub const MAX_DROPPED_ITEM_INSTANCES: usize = 1_024;

/// One item texture; identical sprites share a layer through their index.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DroppedItemSprite {
    pub width: u32,
    pub height: u32,
    pub rgba8: Arc<[u8]>,
}

/// One drawn copy of a sprite: `world_from_item` maps the unit sprite mesh into the world.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DroppedItemInstance {
    pub sprite: u32,
    pub world_from_item: [[f32; 4]; 3],
    pub block_level: u32,
    pub sky_level: u32,
}

/// The frame's dropped items. `sprites_revision` must change whenever `sprites` changes.
#[derive(Clone, Debug, Default, Resource, ExtractResource)]
pub struct DroppedItemScene {
    pub(crate) sprites_revision: u64,
    pub(crate) sprites: Arc<[DroppedItemSprite]>,
    pub(crate) instances: Arc<[DroppedItemInstance]>,
    pub(crate) daylight: f32,
}

impl DroppedItemScene {
    /// Replaces the frame's contents; instances beyond the cap are dropped.
    pub fn publish(
        &mut self,
        sprites_revision: u64,
        sprites: Arc<[DroppedItemSprite]>,
        instances: &[DroppedItemInstance],
        daylight: f32,
    ) {
        if self.sprites_revision != sprites_revision {
            self.sprites_revision = sprites_revision;
            self.sprites = sprites;
        }
        let count = instances.len().min(MAX_DROPPED_ITEM_INSTANCES);
        self.instances = Arc::from(&instances[..count]);
        self.daylight = if daylight.is_finite() {
            daylight.clamp(0.0, 1.0)
        } else {
            1.0
        };
    }

    pub fn clear(&mut self) {
        self.instances = Arc::from([]);
    }

    #[must_use]
    pub fn instance_count(&self) -> usize {
        self.instances.len()
    }
}

/// Builds `world_from_item` from a centre, yaw about +Y (radians), and uniform scale.
#[must_use]
pub fn dropped_item_transform(center: [f32; 3], yaw_radians: f32, scale: f32) -> [[f32; 4]; 3] {
    let (sine, cosine) = yaw_radians.sin_cos();
    [
        [cosine * scale, 0.0, sine * scale, center[0]],
        [0.0, scale, 0.0, center[1]],
        [-sine * scale, 0.0, cosine * scale, center[2]],
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn publish_caps_instances_and_keeps_sprites_until_the_revision_changes() {
        let mut scene = DroppedItemScene::default();
        let sprite = DroppedItemSprite {
            width: 1,
            height: 1,
            rgba8: Arc::from([255_u8; 4]),
        };
        let instance = DroppedItemInstance {
            sprite: 0,
            world_from_item: dropped_item_transform([0.0; 3], 0.0, 1.0),
            block_level: 0,
            sky_level: 15,
        };
        let many = vec![instance; MAX_DROPPED_ITEM_INSTANCES + 5];
        scene.publish(1, Arc::from([sprite]), &many, f32::NAN);
        assert_eq!(scene.instance_count(), MAX_DROPPED_ITEM_INSTANCES);
        assert_eq!(scene.daylight, 1.0);
        scene.publish(1, Arc::from([]), &many[..1], 0.5);
        assert_eq!(scene.sprites.len(), 1);
        scene.publish(2, Arc::from([]), &[], 0.5);
        assert!(scene.sprites.is_empty());
    }

    #[test]
    fn transform_rotates_about_up_and_translates() {
        let rows = dropped_item_transform([1.0, 2.0, 3.0], std::f32::consts::FRAC_PI_2, 2.0);
        // Local +X maps to world -Z at a quarter turn.
        assert!(rows[0][0].abs() < 1e-6 && (rows[2][0] + 2.0).abs() < 1e-6);
        assert_eq!([rows[0][3], rows[1][3], rows[2][3]], [1.0, 2.0, 3.0]);
    }
}
