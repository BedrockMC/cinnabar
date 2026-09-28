//! Flat sprite items extruded one texel deep, in item space.
//!
//! Each slab face shows the sprite unmirrored to a viewer on that face's side, so a held item
//! reads correctly from either side of the character.
use super::ActorRigVertex;

/// Extrudes an RGBA8 sprite into a slab centred on the origin: the longer side spans one unit,
/// the slab is one texel thick, and each edge quad samples its own texel. `uv_rect` is the
/// sprite's `[u0, v0, u1, v1]` region of its texture layer. `None` for an empty or fully
/// transparent sprite. All vertices sit on `bone_index` 0.
#[must_use]
pub fn extruded_sprite_vertices(
    width: usize,
    height: usize,
    rgba8: &[u8],
    uv_rect: [f32; 4],
) -> Option<Vec<ActorRigVertex>> {
    if width == 0 || height == 0 || rgba8.len() != width.checked_mul(height)?.checked_mul(4)? {
        return None;
    }
    let opaque = |column: isize, row: isize| {
        column >= 0
            && row >= 0
            && (column as usize) < width
            && (row as usize) < height
            && rgba8[(row as usize * width + column as usize) * 4 + 3] != 0
    };
    let texel = 1.0 / width.max(height) as f32;
    let half_depth = texel * 0.5;
    let x_at = |column: usize| (column as f32 - width as f32 * 0.5) * texel;
    let y_at = |row: usize| (height as f32 * 0.5 - row as f32) * texel;
    let to_region = |[u, v]: [f32; 2]| {
        [
            uv_rect[0] + (uv_rect[2] - uv_rect[0]) * u,
            uv_rect[1] + (uv_rect[3] - uv_rect[1]) * v,
        ]
    };
    let mut vertices = Vec::new();
    let (x0, x1) = (x_at(0), x_at(width));
    let (y0, y1) = (y_at(height), y_at(0));
    for (z, normal) in [
        (half_depth, [0.0, 0.0, 1.0]),
        (-half_depth, [0.0, 0.0, -1.0]),
    ] {
        let corners = [[x0, y0, z], [x1, y0, z], [x1, y1, z], [x0, y1, z]];
        let uvs = [[0.0, 1.0], [1.0, 1.0], [1.0, 0.0], [0.0, 0.0]].map(to_region);
        push_quad(&mut vertices, corners, uvs, normal);
    }
    let mut any = false;
    for row in 0..height {
        for column in 0..width {
            if !opaque(column as isize, row as isize) {
                continue;
            }
            any = true;
            let (column_i, row_i) = (column as isize, row as isize);
            let uv = to_region([
                (column as f32 + 0.5) / width as f32,
                (row as f32 + 0.5) / height as f32,
            ]);
            let (left, right) = (x_at(column), x_at(column + 1));
            let (top, bottom) = (y_at(row), y_at(row + 1));
            let edges = [
                (
                    opaque(column_i - 1, row_i),
                    [-1.0, 0.0, 0.0],
                    [left, left],
                    [bottom, top],
                ),
                (
                    opaque(column_i + 1, row_i),
                    [1.0, 0.0, 0.0],
                    [right, right],
                    [bottom, top],
                ),
                (
                    opaque(column_i, row_i - 1),
                    [0.0, 1.0, 0.0],
                    [left, right],
                    [top, top],
                ),
                (
                    opaque(column_i, row_i + 1),
                    [0.0, -1.0, 0.0],
                    [left, right],
                    [bottom, bottom],
                ),
            ];
            for (neighbour_opaque, normal, xs, ys) in edges {
                if neighbour_opaque {
                    continue;
                }
                let corners = [
                    [xs[0], ys[0], half_depth],
                    [xs[0], ys[0], -half_depth],
                    [xs[1], ys[1], -half_depth],
                    [xs[1], ys[1], half_depth],
                ];
                push_quad(&mut vertices, corners, [uv; 4], [uv; 4], normal);
            }
        }
    }
    any.then_some(vertices)
}

fn push_quad(
    vertices: &mut Vec<ActorRigVertex>,
    corners: [[f32; 3]; 4],
    uvs: [[f32; 2]; 4],
    back_uvs: [[f32; 2]; 4],
    normal: [f32; 3],
) {
    let edge_a = std::array::from_fn::<f32, 3, _>(|axis| corners[1][axis] - corners[0][axis]);
    let edge_b = std::array::from_fn::<f32, 3, _>(|axis| corners[2][axis] - corners[0][axis]);
    let cross = [
        edge_a[1] * edge_b[2] - edge_a[2] * edge_b[1],
        edge_a[2] * edge_b[0] - edge_a[0] * edge_b[2],
        edge_a[0] * edge_b[1] - edge_a[1] * edge_b[0],
    ];
    let facing = cross[0] * normal[0] + cross[1] * normal[1] + cross[2] * normal[2];
    let order: [usize; 6] = if facing >= 0.0 {
        [0, 1, 2, 0, 2, 3]
    } else {
        [0, 2, 1, 0, 3, 2]
    };
    for index in order {
        vertices.push(ActorRigVertex {
            position: corners[index],
            normal,
            uv: uvs[index],
            back_uv: back_uvs[index],
            bone_index: 0,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::extruded_sprite_vertices;

    const FULL: [f32; 4] = [0.0, 0.0, 1.0, 1.0];

    fn opaque(width: usize, height: usize) -> Vec<u8> {
        vec![255; width * height * 4]
    }

    #[test]
    fn opaque_slab_has_two_faces_and_only_outer_edges() {
        let single = extruded_sprite_vertices(1, 1, &opaque(1, 1), FULL).unwrap();
        assert_eq!(single.len(), 12 + 4 * 6);
        let pair = extruded_sprite_vertices(2, 1, &opaque(2, 1), FULL).unwrap();
        assert_eq!(pair.len(), 12 + 6 * 6);
    }

    #[test]
    fn transparent_or_malformed_sprites_have_no_mesh() {
        assert!(extruded_sprite_vertices(2, 2, &[0; 16], FULL).is_none());
        assert!(extruded_sprite_vertices(2, 2, &[255; 15], FULL).is_none());
        assert!(extruded_sprite_vertices(0, 2, &[], FULL).is_none());
    }

    #[test]
    fn slab_is_one_texel_thick_and_unit_wide() {
        let vertices = extruded_sprite_vertices(16, 16, &opaque(16, 16), FULL).unwrap();
        let (mut min, mut max) = ([f32::MAX; 3], [f32::MIN; 3]);
        for vertex in &vertices {
            for axis in 0..3 {
                min[axis] = min[axis].min(vertex.position[axis]);
                max[axis] = max[axis].max(vertex.position[axis]);
            }
            assert!(vertex.uv.iter().all(|value| (0.0..=1.0).contains(value)));
        }
        assert!((max[0] - min[0] - 1.0).abs() < 1e-6);
        assert!((max[2] - min[2] - 1.0 / 16.0).abs() < 1e-6);
    }

    #[test]
    fn uvs_map_into_the_requested_atlas_region() {
        let region = [0.25, 0.5, 0.5, 0.75];
        for vertex in extruded_sprite_vertices(4, 4, &opaque(4, 4), region).unwrap() {
            assert!((0.25..=0.5).contains(&vertex.uv[0]));
            assert!((0.5..=0.75).contains(&vertex.uv[1]));
        }
    }

    #[test]
    fn faces_show_the_sprite_unmirrored_from_their_own_side() {
        let vertices = extruded_sprite_vertices(4, 4, &opaque(4, 4), FULL).unwrap();
        for triangle in vertices[..12].chunks_exact(3) {
            for vertex in triangle {
                // The front-facing UV of one slab is the other slab's back-facing UV, mirrored in u.
                let other = vertices[..12]
                    .iter()
                    .find(|other| {
                        other.position[..2] == vertex.position[..2]
                            && other.position[2] != vertex.position[2]
                    })
                    .unwrap();
                assert!((vertex.uv[0] - (1.0 - other.uv[0])).abs() < 1e-6);
                assert_eq!(vertex.uv[1], other.uv[1]);
                assert_eq!(vertex.back_uv, other.uv);
            }
        }
    }

    #[test]
    fn every_triangle_winds_counter_clockwise_about_its_normal() {
        let mut sprite = opaque(3, 3);
        sprite[(4 * 4) + 3] = 0;
        for triangle in extruded_sprite_vertices(3, 3, &sprite, FULL)
            .unwrap()
            .chunks_exact(3)
        {
            let a = triangle[0].position;
            let b = triangle[1].position;
            let c = triangle[2].position;
            let ab = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
            let ac = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
            let cross = [
                ab[1] * ac[2] - ab[2] * ac[1],
                ab[2] * ac[0] - ab[0] * ac[2],
                ab[0] * ac[1] - ab[1] * ac[0],
            ];
            let normal = triangle[0].normal;
            assert!(cross[0] * normal[0] + cross[1] * normal[1] + cross[2] * normal[2] > 0.0);
        }
    }
}
