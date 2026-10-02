//! ShieldRenderer GUI branch, not the first-person attachable animation or a flat UV sheet.
//! Matched 1.26.50.26 RVA 0x05e588d0: T(8,10,-10) S(11) Rx(30) Ry(30), model unit 1/16.

use crate::entity::{EntityAssetCompilation, compile_equipment_textures};
use assets::{
    AssetError, EntityGeometry, EntityGeometryBone, EntityGeometryCube, EntityGeometryUv,
    EquipmentTexture, IconSprite,
};
use std::{path::Path, sync::Arc};

mod raster;

pub(super) const IDENTIFIER: &str = "minecraft:shield";
const ROOT_BONE: &str = "shield";
const MODEL_UNIT: f32 = 1.0 / 16.0;
const GUI_TRANSLATION: [f32; 3] = [8.0, 10.0, -10.0];
const GUI_MODEL_SCALE: f32 = 11.0;
const MODEL_PART_HEIGHT: f32 = 24.0;
// Matched DAT_150254be4, IEEE-754 bits of the native 30-degree angle.
const GUI_ROTATION_RADIANS: f32 = f32::from_bits(0x3f06_0a92);
/// Carrier resolution only; native GUI coordinates retain their sixteen-pixel item frame.
const SIDE: usize = 64;
const PIXELS_PER_GUI_PIXEL: f32 = SIDE as f32 / 16.0;
const FACE_CORNERS: [[usize; 4]; 6] = [
    [3, 2, 1, 0],
    [6, 7, 4, 5],
    [7, 3, 0, 4],
    [2, 6, 5, 1],
    [7, 6, 2, 3],
    [0, 1, 5, 4],
];

pub(super) fn compile(
    root: &Path,
    compilation: &EntityAssetCompilation,
) -> Result<Option<IconSprite>, AssetError> {
    let Some(binding) = compilation
        .equipment_bindings
        .iter()
        .find(|binding| binding.identifier.as_ref() == IDENTIFIER)
    else {
        return Ok(None);
    };
    let Some(geometry) = compilation
        .assets
        .geometries
        .iter()
        .find(|geometry| geometry.identifier == binding.geometry.identifier)
    else {
        return Ok(None);
    };
    let textures = compile_equipment_textures(
        root,
        &compilation.assets.sources,
        std::slice::from_ref(binding),
    )?;
    let Some(texture) = textures
        .iter()
        .find(|texture| texture.identifier == binding.texture.identifier)
    else {
        return Ok(None);
    };
    Ok(bake(geometry, texture))
}

fn bake(geometry: &EntityGeometry, texture: &EquipmentTexture) -> Option<IconSprite> {
    // The retail ShieldModel loads its named root. Exotic animated/inherited model-part trees
    // must not silently use an invented transform; those remain an explicit unsupported branch.
    let bone = geometry
        .bones
        .iter()
        .find(|bone| bone.name.as_ref() == ROOT_BONE)?;
    if geometry.inherits.is_some()
        || bone.parent.is_some()
        || bone.never_render == Some(true)
        || !bone.texture_meshes.is_empty()
        || bone
            .rotation
            .is_some_and(|r| r.iter().any(|v| v.get() != 0.0))
        || geometry
            .bones
            .iter()
            .any(|b| b.parent.as_deref() == Some(ROOT_BONE))
        || bone.cubes.is_empty()
    {
        return None;
    }
    let mut pixels = vec![0; SIDE * SIDE * 4];
    for cube in &bone.cubes {
        if cube.rotation.iter().any(|value| value.get() != 0.0) {
            return None;
        }
        append_cube(&mut pixels, geometry, bone, cube, texture);
    }
    Some(IconSprite {
        width: SIDE as u16,
        height: SIDE as u16,
        rgba8: Arc::from(pixels),
    })
}

fn project(authored: [f32; 3]) -> [f32; 2] {
    // ModelPart::loadWithOrientation uses (pivot.x, 24-pivot.y, pivot.z), and its boxes
    // invert Y around that pivot: together this is (x, 24-y, z), without actor-space X mirroring.
    let [x, y, z] = [authored[0], MODEL_PART_HEIGHT - authored[1], authored[2]];
    let (sin, cos) = GUI_ROTATION_RADIANS.sin_cos();
    let [x, z] = [cos * x + sin * z, -sin * x + cos * z];
    let y = cos * y - sin * z;
    [
        GUI_TRANSLATION[0] + x * MODEL_UNIT * GUI_MODEL_SCALE,
        GUI_TRANSLATION[1] + y * MODEL_UNIT * GUI_MODEL_SCALE,
    ]
    .map(|value| value * PIXELS_PER_GUI_PIXEL)
}

fn append_cube(
    pixels: &mut [u8],
    geometry: &EntityGeometry,
    bone: &EntityGeometryBone,
    cube: &EntityGeometryCube,
    texture: &EquipmentTexture,
) {
    let origin = cube.origin.map(|value| value.get());
    let size = cube.size.map(|value| value.get());
    let inflate = cube.inflate.get() + bone.inflate.map_or(0.0, |value| value.get());
    let min: [f32; 3] = std::array::from_fn(|axis| origin[axis] - inflate);
    let max: [f32; 3] = std::array::from_fn(|axis| origin[axis] + size[axis] + inflate);
    let corners = [
        [min[0], min[1], min[2]],
        [max[0], min[1], min[2]],
        [max[0], max[1], min[2]],
        [min[0], max[1], min[2]],
        [min[0], min[1], max[2]],
        [max[0], min[1], max[2]],
        [max[0], max[1], max[2]],
        [min[0], max[1], max[2]],
    ]
    .map(project);
    let mirror = cube.mirror ^ bone.mirror.unwrap_or(false);
    let uvs = face_uvs(cube);
    for (face, corners_index) in FACE_CORNERS.into_iter().enumerate() {
        let Some(uvs) = uvs[face] else {
            continue;
        };
        let mut points = corners_index.map(|index| corners[if mirror { index ^ 1 } else { index }]);
        let mut uvs = uvs.map(|uv| {
            [
                uv[0] / f32::from(geometry.texture_width),
                uv[1] / f32::from(geometry.texture_height),
            ]
        });
        if mirror {
            points.reverse();
            uvs.reverse();
        }
        // Native GUI disables depth testing, but keeps backface culling. Preserve authored
        // cube/face order (the handle precedes the board), no cube-thumbnail light/depth heuristic.
        for triangle in [[0, 1, 2], [0, 2, 3]] {
            raster::triangle(
                pixels,
                triangle.map(|i| points[i]),
                triangle.map(|i| uvs[i]),
                texture,
            );
        }
    }
}

fn face_uvs(cube: &EntityGeometryCube) -> [Option<[[f32; 2]; 4]>; 6] {
    let quad =
        |[u, v]: [f32; 2], [w, h]: [f32; 2]| [[u, v], [u + w, v], [u + w, v + h], [u, v + h]];
    match &cube.uv {
        EntityGeometryUv::Box(uv) => {
            let [u, v] = uv.map(|value| value.get());
            let [x, y, z] = cube.size.map(|value| value.get().trunc());
            [
                [u + z, v + z, x, y],
                [u + z + x + z, v + z, x, y],
                [u, v + z, z, y],
                [u + z + x, v + z, z, y],
                [u + z, v, x, z],
                [u + z + x, v, x, z],
            ]
            .map(|[u, v, w, h]| Some(quad([u, v], [w, h])))
        }
        EntityGeometryUv::Faces(faces) => {
            let faces = [
                &faces.north,
                &faces.south,
                &faces.east,
                &faces.west,
                &faces.up,
                &faces.down,
            ];
            let dimensions = cube.face_uv_dimensions();
            std::array::from_fn(|index| {
                faces[index].as_ref().map(|face| {
                    quad(
                        face.uv.map(|value| value.get()),
                        face.uv_size
                            .map_or(dimensions[index], |size| size.map(|value| value.get())),
                    )
                })
            })
        }
    }
}

#[cfg(test)]
mod tests;
