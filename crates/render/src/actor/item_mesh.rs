//! Item geometry as actor rig vertices on bone 0.
use super::ActorRigVertex;
use crate::item_geometry::{ItemVertex, cube_vertices, extruded_sprite_vertices as extrude};

fn on_bone_zero(vertices: Vec<ItemVertex>) -> Vec<ActorRigVertex> {
    vertices
        .into_iter()
        .map(|vertex| ActorRigVertex {
            position: vertex.position,
            normal: vertex.normal,
            uv: vertex.uv,
            back_uv: vertex.back_uv,
            bone_index: 0,
        })
        .collect()
}

/// A sprite slab (see `item_geometry`); `None` for a malformed pixel buffer.
#[must_use]
pub fn extruded_sprite_vertices(
    width: usize,
    height: usize,
    rgba8: &[u8],
    uv_rect: [f32; 4],
) -> Option<Vec<ActorRigVertex>> {
    extrude(width, height, rgba8, uv_rect).map(on_bone_zero)
}

/// A unit block cube; face `f` samples `face_rects[f]`.
#[must_use]
pub fn textured_cube_vertices(face_rects: [[f32; 4]; 6]) -> Vec<ActorRigVertex> {
    on_bone_zero(cube_vertices(face_rects))
}
