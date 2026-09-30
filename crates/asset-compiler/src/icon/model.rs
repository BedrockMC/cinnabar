//! Offline thumbnails for block items drawn in 3D that are not plain opaque cubes (slabs, stairs,
//! walls, glass): the world visual's isolated template quads through the cube thumbnail's
//! projection, depth-tested and alpha-tested. Provisional: vanilla's GUI tessellation of
//! connected shapes (fences, walls) differs and is not modelled.

use std::{borrow::Cow, sync::Arc};

use assets::{
    BlockFace, BlockVisualId, IconSprite, MATERIAL_FLAG_ALPHA_BLEND, MATERIAL_FLAG_ALPHA_CUTOUT,
    MODEL_TEMPLATE_FLAG_COMPOUND_NEXT, MODEL_TEMPLATE_FLAG_FENCE_NETHER,
    MODEL_TEMPLATE_FLAG_FENCE_WOOD, ModelQuad, NetworkIdMode, RuntimeAssets, VisualKind,
};

use super::cube::{PIXEL_BYTES, Reject};

pub(super) const SIDE: usize = 32;
const TILE: usize = 16;

struct Face<'a> {
    corners: [[f32; 3]; 4],
    uvs: [[f32; 2]; 4],
    tile: Cow<'a, [u8]>,
    blend: bool,
}

pub(super) struct Model<'a> {
    faces: Vec<Face<'a>>,
}

impl<'a> Model<'a> {
    pub(super) fn read(world: &'a RuntimeAssets, visual: BlockVisualId) -> Result<Self, Reject> {
        if visual.0 as usize >= world.visual_count() {
            return Err(Reject::Geometry);
        }
        let block = world.resolve(NetworkIdMode::Sequential, visual.0);
        if !block.is_known() {
            return Err(Reject::Geometry);
        }
        let mut faces = Vec::new();
        match (block.kind(), block.model_template()) {
            (VisualKind::Cube, _) => {
                for face in BlockFace::ALL {
                    let (corners, uvs) = cube_face(face);
                    let (tile, blend) = tile(world, block.face(face).material_id())?;
                    faces.push(Face {
                        corners,
                        uvs,
                        tile: Cow::Borrowed(tile),
                        blend,
                    });
                }
            }
            (VisualKind::Model, Some(template)) => {
                let templates = world.model_templates();
                let first = templates.get(template as usize).ok_or(Reject::Geometry)?;
                // A fence item shows its post with east and west arms (connection mask 2 | 8).
                let parts: &[usize] = if first.flags
                    & (MODEL_TEMPLATE_FLAG_FENCE_WOOD | MODEL_TEMPLATE_FLAG_FENCE_NETHER)
                    != 0
                {
                    &[0, 11]
                } else if first.flags & MODEL_TEMPLATE_FLAG_COMPOUND_NEXT != 0 {
                    &[0, 1]
                } else {
                    &[0]
                };
                for part in parts {
                    let template = templates
                        .get(template as usize + part)
                        .ok_or(Reject::Geometry)?;
                    let start = template.quad_start as usize;
                    let quads = world
                        .model_quads()
                        .get(start..start + template.quad_count as usize)
                        .ok_or(Reject::Geometry)?;
                    for quad in quads {
                        faces.push(model_face(world, quad)?);
                    }
                }
            }
            _ => return Err(Reject::Geometry),
        }
        if faces.is_empty() {
            return Err(Reject::Geometry);
        }
        Ok(Self { faces })
    }

    /// A cutout full cube from six 16x16 tiles in `BlockFace` order.
    pub(super) fn cube(tiles: [Box<[u8]>; 6]) -> Model<'static> {
        let faces = BlockFace::ALL
            .into_iter()
            .zip(tiles)
            .map(|(face, tile)| {
                let (corners, uvs) = cube_face(face);
                Face {
                    corners,
                    uvs,
                    tile: Cow::Owned(tile.into_vec()),
                    blend: false,
                }
            })
            .collect();
        Model { faces }
    }

    pub(super) fn raster(&self) -> IconSprite {
        let mut pixels = vec![0u8; SIDE * SIDE * 4];
        let mut depth = vec![f32::INFINITY; SIDE * SIDE];
        for face in &self.faces {
            let brightness = brightness(face.corners);
            let points = face.corners.map(project);
            for indices in [[0, 1, 2], [0, 2, 3]] {
                triangle(
                    &mut pixels,
                    &mut depth,
                    face,
                    indices.map(|i| points[i]),
                    indices.map(|i| face.uvs[i]),
                    brightness,
                );
            }
        }
        IconSprite {
            width: SIDE as u16,
            height: SIDE as u16,
            rgba8: Arc::from(pixels),
        }
    }
}

fn tile(world: &RuntimeAssets, id: u32) -> Result<(&[u8], bool), Reject> {
    if id == assets::DIAGNOSTIC_MATERIAL {
        return Err(Reject::Material);
    }
    let material = world.materials().get(id as usize).ok_or(Reject::Material)?;
    let alpha = MATERIAL_FLAG_ALPHA_BLEND | MATERIAL_FLAG_ALPHA_CUTOUT;
    // Tints, overlays and rotated UVs need per-biome or per-state data an icon lacks.
    if material.flags & !alpha != 0 {
        return Err(Reject::Material);
    }
    let page = world
        .texture_pages()
        .get(material.texture.page() as usize)
        .ok_or(Reject::Texture)?;
    let mip = page.texture.mips.first().ok_or(Reject::Texture)?;
    if mip.size as usize != TILE || material.texture.layer() >= page.texture.layers {
        return Err(Reject::Texture);
    }
    let start = material.texture.layer() as usize * PIXEL_BYTES;
    let tile = mip
        .rgba8
        .get(start..start + PIXEL_BYTES)
        .ok_or(Reject::Texture)?;
    Ok((tile, material.flags & MATERIAL_FLAG_ALPHA_BLEND != 0))
}

fn model_face<'a>(world: &'a RuntimeAssets, quad: &ModelQuad) -> Result<Face<'a>, Reject> {
    let (tile, blend) = tile(world, quad.material)?;
    Ok(Face {
        tile: Cow::Borrowed(tile),
        corners: quad
            .positions
            .map(|point| point.map(|component| f32::from(component) / 256.0)),
        uvs: quad
            .uvs
            .map(|uv| uv.map(|component| f32::from(component) / 4096.0)),
        blend,
    })
}

/// A full-block face with the cube thumbnail's UV orientation.
fn cube_face(face: BlockFace) -> ([[f32; 3]; 4], [[f32; 2]; 4]) {
    let side_uv = [[0., 1.], [1., 1.], [1., 0.], [0., 0.]];
    match face {
        BlockFace::Up => (
            [[0., 1., 0.], [0., 1., 1.], [1., 1., 1.], [1., 1., 0.]],
            [[0., 0.], [0., 1.], [1., 1.], [1., 0.]],
        ),
        BlockFace::Down => (
            [[0., 0., 1.], [0., 0., 0.], [1., 0., 0.], [1., 0., 1.]],
            [[0., 0.], [0., 1.], [1., 1.], [1., 0.]],
        ),
        BlockFace::South => (
            [[0., 0., 1.], [1., 0., 1.], [1., 1., 1.], [0., 1., 1.]],
            side_uv,
        ),
        BlockFace::North => (
            [[1., 0., 0.], [0., 0., 0.], [0., 1., 0.], [1., 1., 0.]],
            side_uv,
        ),
        BlockFace::West => (
            [[0., 0., 0.], [0., 0., 1.], [0., 1., 1.], [0., 1., 0.]],
            side_uv,
        ),
        BlockFace::East => (
            [[1., 0., 1.], [1., 0., 0.], [1., 1., 0.], [1., 1., 1.]],
            side_uv,
        ),
    }
}

/// The cube thumbnail's side shading, chosen by the face normal's dominant axis.
fn brightness(corners: [[f32; 3]; 4]) -> f32 {
    let [a, b, c] = [corners[0], corners[1], corners[2]];
    let (u, v) = (
        [b[0] - a[0], b[1] - a[1], b[2] - a[2]],
        [c[0] - a[0], c[1] - a[1], c[2] - a[2]],
    );
    let normal = [
        (u[1] * v[2] - u[2] * v[1]).abs(),
        u[2] * v[0] - u[0] * v[2],
        (u[0] * v[1] - u[1] * v[0]).abs(),
    ];
    if normal[1].abs() >= normal[0] && normal[1].abs() >= normal[2] {
        1.
    } else if normal[0] >= normal[2] {
        f32::from_bits(0x3f3ae148)
    } else {
        0.5
    }
}

/// The cube thumbnail's projection at twice its scale, plus a view depth (smaller is nearer).
fn project([x, y, z]: [f32; 3]) -> [f32; 3] {
    let (sx, cx) = (f32::from_bits(0xbeffffff), f32::from_bits(0xbf5db3d7));
    let (sy, cy) = (f32::from_bits(0x3f3504f3), f32::from_bits(0x3f3504f3));
    let rotated_x = cy * x + sy * z;
    let rotated_z = -sy * x + cy * z;
    [
        2. * (1. + 10. * rotated_x),
        2. * (f32::from_bits(0x4147ae14) + 10. * (cx * y - sx * rotated_z)),
        sx * y + cx * rotated_z,
    ]
}

fn edge(a: [f32; 3], b: [f32; 3], p: [f32; 2]) -> f32 {
    (b[0] - a[0]) * (p[1] - a[1]) - (b[1] - a[1]) * (p[0] - a[0])
}

fn triangle(
    pixels: &mut [u8],
    depth: &mut [f32],
    face: &Face<'_>,
    mut p: [[f32; 3]; 3],
    mut uv: [[f32; 2]; 3],
    brightness: f32,
) {
    let area = edge(p[0], p[1], [p[2][0], p[2][1]]);
    if area < 0. {
        p.swap(1, 2);
        uv.swap(1, 2);
    }
    let area = area.abs();
    if !area.is_finite() || area == 0. {
        return;
    }
    for y in 0..SIDE {
        for x in 0..SIDE {
            let sample = [x as f32 + 0.5, y as f32 + 0.5];
            let weights = [
                edge(p[1], p[2], sample) / area,
                edge(p[2], p[0], sample) / area,
                edge(p[0], p[1], sample) / area,
            ];
            if weights.iter().any(|&weight| weight < 0.) {
                continue;
            }
            let z = (0..3).map(|i| weights[i] * p[i][2]).sum::<f32>();
            let target = y * SIDE + x;
            if z >= depth[target] {
                continue;
            }
            let [u, v] = [0, 1].map(|axis| (0..3).map(|i| weights[i] * uv[i][axis]).sum::<f32>());
            let tx = ((u.rem_euclid(1.) * TILE as f32) as usize).min(TILE - 1);
            let ty = ((v.rem_euclid(1.) * TILE as f32) as usize).min(TILE - 1);
            let texel = &face.tile[(ty * TILE + tx) * 4..][..4];
            if texel[3] < 128 && !(face.blend && texel[3] > 0) {
                continue;
            }
            depth[target] = z;
            for c in 0..3 {
                pixels[target * 4 + c] =
                    (f32::from(texel[c]) * brightness).round().clamp(0., 255.) as u8;
            }
            pixels[target * 4 + 3] = if face.blend { texel[3] } else { 255 };
        }
    }
}
