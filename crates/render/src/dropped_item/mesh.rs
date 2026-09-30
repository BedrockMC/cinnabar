use crate::item_geometry::{ItemVertex, cube_vertices, extruded_sprite_vertices};
use bytemuck::{Pod, Zeroable};

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Pod, Zeroable)]
pub struct ItemMeshVertex {
    pub position: [f32; 3],
    /// Normalised into the sprite's GPU layer, not the sprite itself.
    pub uv: [f32; 2],
    pub normal: [f32; 3],
    /// Layer of the shared sprite array this vertex samples.
    pub layer: u32,
    /// Little-endian RGBA8 multiplier applied to the sampled texel.
    pub color: u32,
}

pub const ITEM_MESH_VERTEX_BYTES: usize = std::mem::size_of::<ItemMeshVertex>();
const _: () = assert!(ITEM_MESH_VERTEX_BYTES == 40);

pub const OPAQUE_WHITE: u32 = 0xffff_ffff;

/// Builds a unit cube centred on the origin; face `i` samples `layers[i]` over a `tile`-texel
/// square inside a `layer_side` layer and is multiplied by `colors[i]`.
#[must_use]
pub fn cube_mesh(
    layers: [u32; 6],
    colors: [u32; 6],
    tile: u32,
    layer_side: u32,
) -> Option<Vec<ItemMeshVertex>> {
    if tile == 0 || tile > layer_side {
        return None;
    }
    let extent = tile as f32 / layer_side as f32;
    let vertices = cube_vertices([[0.0, 0.0, extent, extent]; 6]);
    // Six vertices per face, in face order.
    Some(
        vertices
            .chunks_exact(6)
            .zip(layers.into_iter().zip(colors))
            .flat_map(|(face, (layer, color))| {
                face.iter().map(move |vertex| paint(vertex, layer, color))
            })
            .collect(),
    )
}

/// Builds a unit-wide sprite one texel thick, centred on the origin: full front and back faces
/// (the shader discards transparent texels) plus a side quad wherever a solid texel borders
/// empty space. Returns `None` when the pixel buffer does not match the size.
#[must_use]
pub fn extruded_sprite_mesh(
    width: u32,
    height: u32,
    rgba8: &[u8],
    layer_side: u32,
    layer: u32,
) -> Option<Vec<ItemMeshVertex>> {
    if width == 0 || height == 0 || width > layer_side || height > layer_side {
        return None;
    }
    let side = layer_side as f32;
    let rect = [0.0, 0.0, width as f32 / side, height as f32 / side];
    let vertices = extruded_sprite_vertices(width as usize, height as usize, rgba8, rect)?;
    Some(
        vertices
            .iter()
            .map(|vertex| paint(vertex, layer, OPAQUE_WHITE))
            .collect(),
    )
}

fn paint(vertex: &ItemVertex, layer: u32, color: u32) -> ItemMeshVertex {
    ItemMeshVertex {
        position: vertex.position,
        uv: vertex.uv,
        normal: vertex.normal,
        layer,
        color,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pixels(width: u32, height: u32, solid_texels: &[(u32, u32)]) -> Vec<u8> {
        let mut rgba8 = vec![0_u8; (width * height * 4) as usize];
        for &(column, row) in solid_texels {
            let offset = ((row * width + column) * 4) as usize;
            rgba8[offset..offset + 4].copy_from_slice(&[200, 100, 50, 255]);
        }
        rgba8
    }

    #[test]
    fn single_texel_has_two_faces_and_four_edges() {
        let mesh = extruded_sprite_mesh(1, 1, &pixels(1, 1, &[(0, 0)]), 32, 0).unwrap();
        // Two full faces plus four side quads, six vertices each.
        assert_eq!(mesh.len(), 6 * 6);
    }

    #[test]
    fn interior_shared_edges_are_not_emitted() {
        let mesh = extruded_sprite_mesh(2, 1, &pixels(2, 1, &[(0, 0), (1, 0)]), 32, 0).unwrap();
        // Faces + top/bottom for both texels + outer left/right only.
        assert_eq!(mesh.len(), 6 * (2 + 4 + 2));
    }

    #[test]
    fn transparent_sprite_only_has_faces_and_bad_input_is_rejected() {
        assert_eq!(
            extruded_sprite_mesh(2, 2, &pixels(2, 2, &[]), 32, 0)
                .unwrap()
                .len(),
            12
        );
        assert!(extruded_sprite_mesh(2, 2, &[0; 4], 32, 0).is_none());
        assert!(extruded_sprite_mesh(33, 1, &[0; 33 * 4], 32, 0).is_none());
        assert!(extruded_sprite_mesh(0, 1, &[], 32, 0).is_none());
    }

    #[test]
    fn mesh_is_centred_and_one_texel_thick() {
        let mesh = extruded_sprite_mesh(16, 16, &pixels(16, 16, &[(3, 3)]), 32, 0).unwrap();
        let (mut min, mut max) = ([f32::MAX; 3], [f32::MIN; 3]);
        for vertex in &mesh {
            for (axis, value) in vertex.position.iter().enumerate() {
                min[axis] = min[axis].min(*value);
                max[axis] = max[axis].max(*value);
            }
        }
        assert!((min[0] + 0.5).abs() < 1e-6 && (max[0] - 0.5).abs() < 1e-6);
        assert!((max[2] - min[2] - 1.0 / 16.0).abs() < 1e-6);
    }

    #[test]
    fn uvs_index_into_the_padded_layer() {
        let mesh = extruded_sprite_mesh(16, 16, &pixels(16, 16, &[(0, 0)]), 32, 4).unwrap();
        assert!(
            mesh.iter()
                .all(|vertex| vertex.uv.iter().all(|c| (0.0..=0.5).contains(c)))
        );
    }

    #[test]
    fn cube_has_six_faces_with_outward_normals_and_per_face_layers() {
        let mesh = cube_mesh([1, 2, 3, 4, 5, 6], [OPAQUE_WHITE; 6], 16, 32).unwrap();
        assert_eq!(mesh.len(), 36);
        for (face, quad) in mesh.chunks(6).enumerate() {
            assert!(quad.iter().all(|vertex| vertex.layer == face as u32 + 1));
            let centre: [f32; 3] = std::array::from_fn(|axis| {
                quad.iter().map(|vertex| vertex.position[axis]).sum::<f32>() / 6.0
            });
            let normal = quad[0].normal;
            let dot: f32 = (0..3).map(|axis| centre[axis] * normal[axis]).sum();
            assert!(dot > 0.1, "face {face} normal points inward");
        }
        assert!(cube_mesh([0; 6], [0; 6], 33, 32).is_none());
    }
}
