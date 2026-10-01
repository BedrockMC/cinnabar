//! Polygon meshes embedded in serialized persona skin geometry.
use serde_json::Value;

use super::{MAX_SKIN_GEOMETRY_VERTICES, vector};

/// One authored polygon corner, before conversion into the actor coordinate frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SkinPolyVertex {
    pub position: [f32; 3],
    pub normal: [f32; 3],
    pub uv: [f32; 2],
}

/// A bone's polygon mesh, expanded into triangles with independent corner attributes.
#[derive(Clone, Debug, PartialEq)]
pub struct SkinPolyMesh {
    pub normalized_uvs: bool,
    pub vertices: Box<[SkinPolyVertex]>,
}

/// Reads one optional mesh for each bone accepted by the skin parser.
pub(super) fn bone_meshes(value: Option<&Value>) -> Vec<Option<SkinPolyMesh>> {
    value
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|bone| {
            let name = bone.get("name")?.as_str()?;
            if name.is_empty() || name.chars().any(char::is_control) {
                return None;
            }
            Some(parse_mesh(bone.get("poly_mesh")))
        })
        .collect()
}

/// Skips malformed polygons while retaining valid neighbours and their attribute indices.
fn parse_mesh(value: Option<&Value>) -> Option<SkinPolyMesh> {
    let mesh = value?.as_object()?;
    let positions = mesh.get("positions")?.as_array()?;
    let normals = mesh.get("normals")?.as_array()?;
    let uvs = mesh.get("uvs")?.as_array()?;
    let corner = |indices: [usize; 3]| -> Option<SkinPolyVertex> {
        Some(SkinPolyVertex {
            position: vector::<3>(positions.get(indices[0]))?.map(|value| value.get()),
            normal: vector::<3>(normals.get(indices[1]))?.map(|value| value.get()),
            uv: vector::<2>(uvs.get(indices[2]))?.map(|value| value.get()),
        })
    };
    let mut vertices = Vec::new();
    match mesh.get("polys")? {
        Value::Array(polygons) => {
            for polygon in polygons {
                let Some(indices) = polygon.as_array().filter(|p| matches!(p.len(), 3 | 4)) else {
                    continue;
                };
                let corners = indices
                    .iter()
                    .map(|indices| {
                        let indices = indices.as_array()?;
                        if indices.len() != 3 {
                            return None;
                        }
                        corner([
                            usize::try_from(indices[0].as_u64()?).ok()?,
                            usize::try_from(indices[1].as_u64()?).ok()?,
                            usize::try_from(indices[2].as_u64()?).ok()?,
                        ])
                    })
                    .collect::<Option<Vec<_>>>();
                if let Some(corners) = corners {
                    append_polygon(&mut vertices, &corners)?;
                }
            }
        }
        Value::String(mode) => {
            let count = match mode.as_str() {
                "tri_list" => 3,
                "quad_list" => 4,
                _ => return None,
            };
            if positions.len() != normals.len()
                || positions.len() != uvs.len()
                || !positions.len().is_multiple_of(count)
            {
                return None;
            }
            for first in (0..positions.len()).step_by(count) {
                let corners = (first..first + count)
                    .map(|index| corner([index; 3]))
                    .collect::<Option<Vec<_>>>();
                if let Some(corners) = corners {
                    append_polygon(&mut vertices, &corners)?;
                }
            }
        }
        _ => return None,
    }
    Some(SkinPolyMesh {
        normalized_uvs: mesh
            .get("normalized_uvs")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        vertices: vertices.into(),
    })
}

/// Splits quads on the first-to-third diagonal without changing authored winding.
fn append_polygon(vertices: &mut Vec<SkinPolyVertex>, corners: &[SkinPolyVertex]) -> Option<()> {
    let added = if corners.len() == 4 { 6 } else { 3 };
    if vertices.len().saturating_add(added) > MAX_SKIN_GEOMETRY_VERTICES {
        return None;
    }
    vertices.extend_from_slice(&corners[..3]);
    if corners.len() == 4 {
        vertices.extend([corners[0], corners[2], corners[3]]);
    }
    Some(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// Independent UV indices and a bad neighbouring polygon must not erase the good triangle.
    #[test]
    fn corner_indices_are_independent_and_bad_polygons_are_skipped() {
        let value = json!({
            "positions": [[0,0,0], [1,0,0], [0,1,0]],
            "normals": [[0,0,1]], "uvs": [[0,0], [1,0], [0,1]],
            "polys": [[[0,0,2],[1,0,0],[2,0,1]], [[0,0,0],[99,0,1],[2,0,2]]]
        });
        let mesh = parse_mesh(Some(&value)).unwrap();
        assert_eq!(mesh.vertices.len(), 3);
        assert_eq!(mesh.vertices[0].uv, [0.0, 1.0]);
        assert_eq!(mesh.vertices[1].position, [1.0, 0.0, 0.0]);
    }

    /// Implicit lists require one normal and UV per position and complete primitives.
    #[test]
    fn implicit_lists_require_matching_complete_attribute_arrays() {
        let mut value = json!({
            "positions": [[0,0,0], [1,0,0], [0,1,0]],
            "normals": [[0,0,1], [0,0,1], [0,0,1]],
            "uvs": [[0,0], [1,0], [0,1]], "polys": "tri_list"
        });
        assert_eq!(parse_mesh(Some(&value)).unwrap().vertices.len(), 3);
        value["polys"] = json!("quad_list");
        assert!(parse_mesh(Some(&value)).is_none());
        value["polys"] = json!("tri_list");
        value["normals"] = json!([[0, 0, 1]]);
        assert!(parse_mesh(Some(&value)).is_none());
    }
}
