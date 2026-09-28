use bytemuck::{Pod, Zeroable};

/// Sprite texel with any alpha counts as solid for extrusion.
const fn solid(alpha: u8) -> bool {
    alpha != 0
}

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Pod, Zeroable)]
pub struct ItemMeshVertex {
    pub position: [f32; 3],
    /// Normalised into the sprite's GPU layer, not the sprite itself.
    pub uv: [f32; 2],
    pub normal: [f32; 3],
}

pub const ITEM_MESH_VERTEX_BYTES: usize = std::mem::size_of::<ItemMeshVertex>();
const _: () = assert!(ITEM_MESH_VERTEX_BYTES == 32);

fn quad(
    vertices: &mut Vec<ItemMeshVertex>,
    corners: [[f32; 3]; 4],
    uvs: [[f32; 2]; 4],
    normal: [f32; 3],
) {
    for index in [0, 1, 2, 0, 2, 3] {
        vertices.push(ItemMeshVertex {
            position: corners[index],
            uv: uvs[index],
            normal,
        });
    }
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
) -> Option<Vec<ItemMeshVertex>> {
    if width == 0 || height == 0 || width > layer_side || height > layer_side {
        return None;
    }
    if rgba8.len()
        != (width as usize)
            .checked_mul(height as usize)?
            .checked_mul(4)?
    {
        return None;
    }
    let (w, h, side) = (width as f32, height as f32, layer_side as f32);
    // One texel of depth in the sprite's own units; the sprite spans 1.0 across its width.
    let half_depth = 0.5 / w;
    let x_at = |column: f32| column / w - 0.5;
    let y_at = |row: f32| 0.5 * h / w - row / w;
    let uv = |column: f32, row: f32| [column / side, row / side];
    let is_solid = |column: i64, row: i64| {
        column >= 0
            && row >= 0
            && column < i64::from(width)
            && row < i64::from(height)
            && solid(rgba8[((row as usize) * width as usize + column as usize) * 4 + 3])
    };

    let mut vertices = Vec::new();
    let (left, right, top, bottom) = (x_at(0.0), x_at(w), y_at(0.0), y_at(h));
    let face_uvs = [uv(0.0, 0.0), uv(w, 0.0), uv(w, h), uv(0.0, h)];
    // Front faces +Z with the sprite's top edge up; the back lists X mirrored so U still reads
    // left to right for a viewer behind it.
    quad(
        &mut vertices,
        [
            [left, top, half_depth],
            [right, top, half_depth],
            [right, bottom, half_depth],
            [left, bottom, half_depth],
        ],
        face_uvs,
        [0.0, 0.0, 1.0],
    );
    quad(
        &mut vertices,
        [
            [right, top, -half_depth],
            [left, top, -half_depth],
            [left, bottom, -half_depth],
            [right, bottom, -half_depth],
        ],
        face_uvs,
        [0.0, 0.0, -1.0],
    );
    for row in 0..i64::from(height) {
        for column in 0..i64::from(width) {
            if !is_solid(column, row) {
                continue;
            }
            let (c, r) = (column as f32, row as f32);
            let centre = uv(c + 0.5, r + 0.5);
            let (x0, x1, y0, y1) = (x_at(c), x_at(c + 1.0), y_at(r), y_at(r + 1.0));
            let flat = [centre; 4];
            if !is_solid(column, row - 1) {
                quad(
                    &mut vertices,
                    [
                        [x0, y0, half_depth],
                        [x1, y0, half_depth],
                        [x1, y0, -half_depth],
                        [x0, y0, -half_depth],
                    ],
                    flat,
                    [0.0, 1.0, 0.0],
                );
            }
            if !is_solid(column, row + 1) {
                quad(
                    &mut vertices,
                    [
                        [x0, y1, -half_depth],
                        [x1, y1, -half_depth],
                        [x1, y1, half_depth],
                        [x0, y1, half_depth],
                    ],
                    flat,
                    [0.0, -1.0, 0.0],
                );
            }
            if !is_solid(column - 1, row) {
                quad(
                    &mut vertices,
                    [
                        [x0, y0, -half_depth],
                        [x0, y1, -half_depth],
                        [x0, y1, half_depth],
                        [x0, y0, half_depth],
                    ],
                    flat,
                    [-1.0, 0.0, 0.0],
                );
            }
            if !is_solid(column + 1, row) {
                quad(
                    &mut vertices,
                    [
                        [x1, y0, half_depth],
                        [x1, y1, half_depth],
                        [x1, y1, -half_depth],
                        [x1, y0, -half_depth],
                    ],
                    flat,
                    [1.0, 0.0, 0.0],
                );
            }
        }
    }
    Some(vertices)
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
        let mesh = extruded_sprite_mesh(1, 1, &pixels(1, 1, &[(0, 0)]), 32).unwrap();
        // Two full faces plus four side quads, six vertices each.
        assert_eq!(mesh.len(), 6 * 6);
    }

    #[test]
    fn interior_shared_edges_are_not_emitted() {
        let mesh = extruded_sprite_mesh(2, 1, &pixels(2, 1, &[(0, 0), (1, 0)]), 32).unwrap();
        // Faces + top/bottom for both texels + outer left/right only.
        assert_eq!(mesh.len(), 6 * (2 + 4 + 2));
    }

    #[test]
    fn transparent_sprite_only_has_faces_and_bad_input_is_rejected() {
        assert_eq!(
            extruded_sprite_mesh(2, 2, &pixels(2, 2, &[]), 32)
                .unwrap()
                .len(),
            12
        );
        assert!(extruded_sprite_mesh(2, 2, &[0; 4], 32).is_none());
        assert!(extruded_sprite_mesh(33, 1, &vec![0; 33 * 4], 32).is_none());
        assert!(extruded_sprite_mesh(0, 1, &[], 32).is_none());
    }

    #[test]
    fn mesh_is_centred_and_one_texel_thick() {
        let mesh = extruded_sprite_mesh(16, 16, &pixels(16, 16, &[(3, 3)]), 32).unwrap();
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
        let mesh = extruded_sprite_mesh(16, 16, &pixels(16, 16, &[(0, 0)]), 32).unwrap();
        assert!(
            mesh.iter()
                .all(|vertex| vertex.uv.iter().all(|c| (0.0..=0.5).contains(c)))
        );
    }
}
