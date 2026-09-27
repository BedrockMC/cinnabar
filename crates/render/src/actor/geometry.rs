use assets::{EntityGeometryCube, EntityGeometryFaceUv, EntityGeometryUv};
use bevy::math::Vec3;

use super::rig::{ActorRigGeometryError, ActorRigVertex};

pub(super) fn append_entity_cube_vertices(
    vertices: &mut Vec<ActorRigVertex>,
    cube: &EntityGeometryCube,
    bone_index: u32,
    texture_size: (u16, u16),
    bone_mirror: bool,
    bone_inflate: f32,
) -> Result<(), ActorRigGeometryError> {
    let origin = cube.origin.map(|value| value.get());
    let size = cube.size.map(|value| value.get());
    let inflate = cube.inflate.get() + bone_inflate;
    if origin
        .iter()
        .chain(size.iter())
        .chain([inflate].iter())
        .any(|value| !value.is_finite())
        || size.iter().any(|value| *value < 0.0)
        || texture_size.0 == 0
        || texture_size.1 == 0
    {
        return Err(ActorRigGeometryError::InvalidAssetGeometry);
    }
    let zero_axes: Vec<_> = size
        .iter()
        .enumerate()
        .filter(|(_, value)| **value == 0.0)
        .map(|(axis, _)| axis)
        .collect();
    if zero_axes.len() > 1 || (!zero_axes.is_empty() && inflate != 0.0) {
        return Err(ActorRigGeometryError::InvalidAssetGeometry);
    }
    let min = std::array::from_fn(|axis| (origin[axis] - inflate) / 16.0);
    let max = std::array::from_fn(|axis| (origin[axis] + size[axis] + inflate) / 16.0);
    if (0..3).any(|axis| size[axis] != 0.0 && min[axis] >= max[axis]) {
        return Err(ActorRigGeometryError::InvalidAssetGeometry);
    }
    let mut corners = cuboid_corners(min, max);
    let pivot = cube.pivot.map(|value| value.get() / 16.0);
    let rotation = cube.rotation.map(|value| value.get());
    if rotation.iter().any(|value| *value != 0.0) {
        for corner in &mut corners {
            *corner = rotate_euler_around(*corner, pivot, rotation)
                .ok_or(ActorRigGeometryError::InvalidAssetGeometry)?;
        }
    }
    let mirror = cube.mirror ^ bone_mirror;
    let face_uvs = entity_face_uvs(&cube.uv, size, texture_size, mirror)?;
    let faces = [
        [0, 2, 1, 0, 3, 2],
        [5, 6, 4, 4, 6, 7],
        [4, 3, 0, 4, 7, 3],
        [1, 2, 5, 5, 2, 6],
        [3, 7, 2, 2, 7, 6],
        [4, 0, 5, 5, 0, 1],
    ];
    if let Some(axis) = zero_axes.first() {
        let (front, back) = [(2, 3), (4, 5), (0, 1)][*axis];
        let (Some(front_uv), Some(back_uv)) = (face_uvs[front], face_uvs[back]) else {
            return Err(ActorRigGeometryError::InvalidAssetGeometry);
        };
        // Canonical quad topology only for authored planes. Mirroring changes
        // UVs, never the selected physical front side or geometry winding.
        let quads = [
            [0, 1, 2, 3],
            [5, 4, 7, 6],
            [4, 0, 3, 7],
            [1, 5, 6, 2],
            [3, 2, 6, 7],
            [4, 5, 1, 0],
        ];
        let front_quad = quads[front];
        let back_quad = quads[back];
        let normal = triangle_normal(
            corners[front_quad[0]],
            corners[front_quad[2]],
            corners[front_quad[1]],
        );
        // One physical quad: the opposing authored face supplies UVs only.
        // Corner correspondence is geometric, not opposing-array ordinal order.
        for index in [0, 2, 1, 0, 3, 2] {
            let corner = front_quad[index];
            let opposite = back_quad
                .iter()
                .position(|back| corners[*back] == corners[corner])
                .ok_or(ActorRigGeometryError::InvalidAssetGeometry)?;
            vertices.push(ActorRigVertex {
                position: corners[corner],
                normal,
                uv: front_uv[index],
                back_uv: back_uv[opposite],
                bone_index,
            });
        }
        return Ok(());
    }
    for (face, uv) in faces.into_iter().zip(face_uvs) {
        let Some(uv) = uv else {
            continue;
        };
        let indices = if mirror {
            [face[0], face[2], face[1], face[3], face[5], face[4]]
        } else {
            face
        };
        let face_uv = [uv[0], uv[2], uv[1], uv[0], uv[3], uv[2]];
        let normal = triangle_normal(
            corners[indices[0]],
            corners[indices[1]],
            corners[indices[2]],
        );
        vertices.extend(
            indices
                .into_iter()
                .zip(face_uv)
                .map(|(corner, uv)| ActorRigVertex {
                    position: corners[corner],
                    normal,
                    uv,
                    back_uv: uv,
                    bone_index,
                }),
        );
    }
    Ok(())
}

type FaceUvQuad = [[f32; 2]; 4];

fn entity_face_uvs(
    uv: &EntityGeometryUv,
    size: [f32; 3],
    texture_size: (u16, u16),
    mirror: bool,
) -> Result<[Option<FaceUvQuad>; 6], ActorRigGeometryError> {
    let (width, height) = (f32::from(texture_size.0), f32::from(texture_size.1));
    let quad = |origin: [f32; 2], dimensions: [f32; 2]| {
        let mut left = origin[0] / width;
        let mut right = (origin[0] + dimensions[0]) / width;
        if mirror {
            std::mem::swap(&mut left, &mut right);
        }
        let top = origin[1] / height;
        let bottom = (origin[1] + dimensions[1]) / height;
        [[left, top], [right, top], [right, bottom], [left, bottom]]
    };
    let result = match uv {
        EntityGeometryUv::Box(origin) => {
            let [u, v] = origin.map(|value| value.get());
            let [x, y, z] = size;
            [
                Some(quad([u + z, v + z], [x, y])),
                Some(quad([u + z + x + z, v + z], [x, y])),
                Some(quad([u + z + x, v + z], [z, y])),
                Some(quad([u, v + z], [z, y])),
                Some(quad([u + z, v], [x, z])),
                Some(quad([u + z + x, v], [x, z])),
            ]
        }
        EntityGeometryUv::Faces(faces) => [
            face_uv_quad(faces.north.as_ref(), &quad),
            face_uv_quad(faces.south.as_ref(), &quad),
            face_uv_quad(faces.west.as_ref(), &quad),
            face_uv_quad(faces.east.as_ref(), &quad),
            face_uv_quad(faces.up.as_ref(), &quad),
            face_uv_quad(faces.down.as_ref(), &quad),
        ],
    };
    if result
        .iter()
        .flatten()
        .flatten()
        .flatten()
        .any(|value| !value.is_finite())
    {
        return Err(ActorRigGeometryError::InvalidAssetGeometry);
    }
    Ok(result)
}

fn face_uv_quad(
    face: Option<&EntityGeometryFaceUv>,
    quad: &impl Fn([f32; 2], [f32; 2]) -> FaceUvQuad,
) -> Option<FaceUvQuad> {
    face.map(|face| {
        quad(
            face.uv.map(|value| value.get()),
            face.uv_size
                .map_or([1.0, 1.0], |size| size.map(|value| value.get())),
        )
    })
}

fn cuboid_corners(min: [f32; 3], max: [f32; 3]) -> [[f32; 3]; 8] {
    [
        [min[0], min[1], min[2]],
        [max[0], min[1], min[2]],
        [max[0], max[1], min[2]],
        [min[0], max[1], min[2]],
        [min[0], min[1], max[2]],
        [max[0], min[1], max[2]],
        [max[0], max[1], max[2]],
        [min[0], max[1], max[2]],
    ]
}

fn rotate_euler_around(point: [f32; 3], pivot: [f32; 3], degrees: [f32; 3]) -> Option<[f32; 3]> {
    if point
        .iter()
        .chain(pivot.iter())
        .chain(degrees.iter())
        .any(|value| !value.is_finite())
    {
        return None;
    }
    let [x, y, z] = degrees.map(|value| value.to_radians());
    let (sx, cx) = x.sin_cos();
    let (sy, cy) = y.sin_cos();
    let (sz, cz) = z.sin_cos();
    let mut value = std::array::from_fn(|axis| point[axis] - pivot[axis]);
    value = [
        value[0],
        value[1] * cx - value[2] * sx,
        value[1] * sx + value[2] * cx,
    ];
    value = [
        value[0] * cy + value[2] * sy,
        value[1],
        -value[0] * sy + value[2] * cy,
    ];
    value = [
        value[0] * cz - value[1] * sz,
        value[0] * sz + value[1] * cz,
        value[2],
    ];
    Some(std::array::from_fn(|axis| value[axis] + pivot[axis]))
}

pub(super) fn cuboid_vertices(
    min: [f32; 3],
    max: [f32; 3],
    bone_index: u32,
) -> Vec<ActorRigVertex> {
    let corners = cuboid_corners(min, max);
    let faces = [
        [0, 2, 1, 0, 3, 2],
        [5, 6, 4, 4, 6, 7],
        [4, 3, 0, 4, 7, 3],
        [1, 2, 5, 5, 2, 6],
        [3, 7, 2, 2, 7, 6],
        [4, 0, 5, 5, 0, 1],
    ];
    let uv = [
        [0.0, 0.0],
        [1.0, 1.0],
        [1.0, 0.0],
        [0.0, 0.0],
        [0.0, 1.0],
        [1.0, 1.0],
    ];
    faces
        .into_iter()
        .flat_map(|face| {
            let normal = triangle_normal(corners[face[0]], corners[face[1]], corners[face[2]]);
            face.into_iter()
                .zip(uv)
                .map(move |(corner, uv)| ActorRigVertex {
                    position: corners[corner],
                    normal,
                    uv,
                    back_uv: uv,
                    bone_index,
                })
        })
        .collect()
}

pub(super) fn triangle_normal(first: [f32; 3], second: [f32; 3], third: [f32; 3]) -> [f32; 3] {
    let left = Vec3::from_array(second) - Vec3::from_array(first);
    let right = Vec3::from_array(third) - Vec3::from_array(first);
    left.cross(right).normalize_or_zero().to_array()
}

#[cfg(test)]
mod tests {
    use super::*;
    use assets::EntityGeometryScalar;
    fn scalar(value: f32) -> EntityGeometryScalar {
        EntityGeometryScalar::new(value).unwrap()
    }
    fn cube(axis: usize, mirror: bool) -> EntityGeometryCube {
        let mut size = [2.0, 3.0, 4.0];
        size[axis] = 0.0;
        EntityGeometryCube {
            origin: [scalar(0.0); 3],
            size: size.map(scalar),
            pivot: [scalar(0.0); 3],
            rotation: [scalar(0.0); 3],
            uv: EntityGeometryUv::Box([scalar(0.0); 2]),
            inflate: scalar(0.0),
            mirror,
        }
    }
    #[test]
    fn every_planar_axis_emits_one_quad_with_finite_opposed_uvs_and_normal() {
        for axis in 0..3 {
            for mirror in [false, true] {
                let mut vertices = Vec::new();
                append_entity_cube_vertices(
                    &mut vertices,
                    &cube(axis, mirror),
                    0,
                    (16, 16),
                    false,
                    0.0,
                )
                .unwrap();
                assert_eq!(vertices.len(), 6);
                assert_eq!(vertices[0].normal[axis], [-1.0, 1.0, -1.0][axis]);
                assert!(vertices.iter().all(|vertex| {
                    vertex.position[axis] == 0.0
                        && vertex
                            .uv
                            .iter()
                            .chain(vertex.back_uv.iter())
                            .all(|value| value.is_finite())
                        && (Vec3::from_array(vertex.normal).length() - 1.0).abs() < 0.0001
                }));
                assert!(vertices.iter().any(|vertex| vertex.uv != vertex.back_uv));
                assert_eq!(vertices[0].position, vertices[3].position);
                assert_eq!(vertices[0].back_uv, vertices[3].back_uv);
                assert_eq!(vertices[1].position, vertices[5].position);
                assert_eq!(vertices[1].back_uv, vertices[5].back_uv);
                let mut corners = Vec::new();
                for vertex in &vertices {
                    if let Some(existing) = corners
                        .iter()
                        .find(|existing: &&ActorRigVertex| existing.position == vertex.position)
                    {
                        assert_eq!(existing.uv, vertex.uv);
                        assert_eq!(existing.back_uv, vertex.back_uv);
                    } else {
                        corners.push(*vertex);
                    }
                }
                assert_eq!(corners.len(), 4);
                let mut equivalent = Vec::new();
                append_entity_cube_vertices(
                    &mut equivalent,
                    &cube(axis, !mirror),
                    0,
                    (16, 16),
                    true,
                    0.0,
                )
                .unwrap();
                assert_eq!(equivalent, vertices);
                let mut rotated_cube = cube(axis, mirror);
                rotated_cube.rotation = [scalar(30.0), scalar(45.0), scalar(60.0)];
                let mut rotated = Vec::new();
                append_entity_cube_vertices(&mut rotated, &rotated_cube, 0, (16, 16), false, 0.0)
                    .unwrap();
                for (before, after) in vertices.iter().zip(rotated) {
                    assert_eq!(before.uv, after.uv);
                    assert_eq!(before.back_uv, after.back_uv);
                    assert!((Vec3::from_array(after.normal).length() - 1.0).abs() < 0.0001);
                }
            }
        }
    }
    #[test]
    fn planar_geometry_rejects_lines_points_inflate_and_missing_opposed_face() {
        let mut cube = cube(0, false);
        for size in [[0.0, 0.0, 3.0], [0.0; 3], [-1.0, 2.0, 3.0]] {
            cube.size = size.map(scalar);
            assert!(
                append_entity_cube_vertices(&mut Vec::new(), &cube, 0, (16, 16), false, 0.0)
                    .is_err()
            );
        }
        cube = self::cube(0, false);
        cube.inflate = scalar(0.1);
        assert!(
            append_entity_cube_vertices(&mut Vec::new(), &cube, 0, (16, 16), false, 0.0).is_err()
        );
        cube.inflate = scalar(0.0);
        cube.uv = EntityGeometryUv::Faces(assets::EntityGeometryFaceUvs {
            north: None,
            south: None,
            east: None,
            west: Some(EntityGeometryFaceUv {
                uv: [scalar(0.0); 2],
                uv_size: Some([scalar(4.0), scalar(3.0)]),
            }),
            up: None,
            down: None,
        });
        assert!(
            append_entity_cube_vertices(&mut Vec::new(), &cube, 0, (16, 16), false, 0.0).is_err()
        );
    }
    #[test]
    fn explicit_planar_faces_match_opposed_uvs_by_physical_corner_after_rotation() {
        let mut cube = cube(0, false);
        let face = |u| EntityGeometryFaceUv {
            uv: [scalar(u), scalar(0.0)],
            uv_size: Some([scalar(4.0), scalar(3.0)]),
        };
        cube.uv = EntityGeometryUv::Faces(assets::EntityGeometryFaceUvs {
            north: None,
            south: None,
            east: Some(face(8.0)),
            west: Some(face(0.0)),
            up: None,
            down: None,
        });
        let mut unrotated = Vec::new();
        append_entity_cube_vertices(&mut unrotated, &cube, 7, (16, 16), false, 0.0).unwrap();
        assert_eq!(unrotated[0].position, [0.0, 0.0, 0.25]);
        assert_eq!(unrotated[0].uv, [0.0, 0.0]);
        assert_eq!(unrotated[0].back_uv, [0.75, 0.0]);
        assert_eq!(unrotated[2].position, [0.0, 0.0, 0.0]);
        assert_eq!(unrotated[2].uv, [0.25, 0.0]);
        assert_eq!(unrotated[2].back_uv, [0.5, 0.0]);
        cube.rotation = [scalar(30.0), scalar(45.0), scalar(60.0)];
        let mut rotated = Vec::new();
        append_entity_cube_vertices(&mut rotated, &cube, 7, (16, 16), false, 0.0).unwrap();
        for (old, new) in unrotated.iter().zip(&rotated) {
            assert_eq!(old.uv, new.uv);
            assert_eq!(old.back_uv, new.back_uv);
            assert_eq!(new.bone_index, 7);
            assert!((Vec3::from_array(new.normal).length() - 1.0).abs() < 0.0001);
        }
    }
}
