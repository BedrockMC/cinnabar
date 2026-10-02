//! Offline opaque-cube thumbnails. Shading is an authored provisional policy,
//! not a claim of complete inventory presentation parity.
use assets::{
    AssetError, BlockFace, BlockFlags, BlockVisualId, IconSprite, NetworkIdMode, RuntimeAssets,
    VisualKind, VisualSupport,
};
use sha2::{Digest, Sha256};
use std::sync::Arc;

pub(super) const POLICY: &str = "opaque-cube-thumbnail-v1";
pub(super) const PIXEL_BYTES: usize =
    assets::BLOCK_ITEM_FACE_SIDE as usize * assets::BLOCK_ITEM_FACE_SIDE as usize * 4;
type FaceSpec = (BlockFace, [[f32; 3]; 4], [[f32; 2]; 4], f32);
const FACES: [FaceSpec; 3] = [
    (
        BlockFace::Up,
        [[0., 1., 0.], [0., 1., 1.], [1., 1., 1.], [1., 1., 0.]],
        [[0., 0.], [0., 1.], [1., 1.], [1., 0.]],
        1.,
    ),
    (
        BlockFace::South,
        [[0., 0., 1.], [1., 0., 1.], [1., 1., 1.], [0., 1., 1.]],
        [[0., 1.], [1., 1.], [1., 0.], [0., 0.]],
        0.5,
    ),
    (
        BlockFace::West,
        [[0., 0., 0.], [0., 0., 1.], [0., 1., 1.], [0., 1., 0.]],
        [[0., 1.], [1., 1.], [1., 0.], [0., 0.]],
        f32::from_bits(0x3f3ae148),
    ),
];

#[derive(Clone, Copy, Debug)]
pub(super) enum Reject {
    Geometry = 0,
    Material = 1,
    Texture = 2,
    Alpha = 3,
}

pub(super) fn invalid(detail: &str) -> AssetError {
    AssetError::InvalidCompiledAssets {
        detail: detail.into(),
    }
}

pub(super) fn validate_world(
    world: &RuntimeAssets,
    manifest: [u8; 32],
    count: usize,
) -> Result<(), AssetError> {
    let registry: [u8; 32] = Sha256::digest(include_bytes!(
        "../../../assets/data/block-registry-v2193.bin"
    ))
    .into();
    let provenance = world.provenance();
    if !provenance.is_complete()
        || provenance.source_manifest_sha256 != manifest
        || provenance.block_registry_sha256 != registry
        || world.visual_count() != count
    {
        return Err(invalid("block icon world/entity identities do not match"));
    }
    Ok(())
}

pub(super) struct Cube<'a> {
    tiles: [&'a [u8]; 6],
    locations: [(u32, u32); 6],
    digest: [u8; 32],
}

impl<'a> Cube<'a> {
    pub(super) fn read(world: &'a RuntimeAssets, visual: BlockVisualId) -> Result<Self, Reject> {
        if visual.0 as usize >= world.visual_count() {
            return Err(Reject::Geometry);
        }
        let block = world.resolve(NetworkIdMode::Sequential, visual.0);
        if !block.is_known()
            || block.kind() != VisualKind::Cube
            || block.support() != VisualSupport::Exact
            || block.flags() != (BlockFlags::CUBE_GEOMETRY | BlockFlags::OCCLUDES_FULL_FACE)
            || block.model_template().is_some()
            || block.animation().is_some()
        {
            return Err(Reject::Geometry);
        }
        let mut tiles = [&[][..]; 6];
        let mut locations = [(0, 0); 6];
        for face in BlockFace::ALL {
            let id = block.face(face).material_id();
            if id == assets::DIAGNOSTIC_MATERIAL {
                return Err(Reject::Material);
            }
            let material = world.materials().get(id as usize).ok_or(Reject::Material)?;
            if material.flags != 0 || material.animation != assets::NO_ANIMATION {
                return Err(Reject::Material);
            }
            let page = world
                .texture_pages()
                .get(material.texture.page() as usize)
                .ok_or(Reject::Texture)?;
            let mip = page.texture.mips.first().ok_or(Reject::Texture)?;
            if mip.size != u32::from(assets::BLOCK_ITEM_FACE_SIDE)
                || material.texture.layer() >= page.texture.layers
            {
                return Err(Reject::Texture);
            }
            let start = (material.texture.layer() as usize)
                .checked_mul(PIXEL_BYTES)
                .ok_or(Reject::Texture)?;
            let end = start.checked_add(PIXEL_BYTES).ok_or(Reject::Texture)?;
            let tile = mip.rgba8.get(start..end).ok_or(Reject::Texture)?;
            if !tile.chunks_exact(4).all(|pixel| pixel[3] == 255) {
                return Err(Reject::Alpha);
            }
            tiles[face as usize] = tile;
            locations[face as usize] = (id, material.texture.raw());
        }
        let mut hash = Sha256::new();
        hash.update(POLICY.as_bytes());
        for (face, corners, uv, brightness) in FACES {
            hash.update([face as u8]);
            for point in corners {
                for component in point {
                    hash.update(component.to_le_bytes());
                }
            }
            for point in uv {
                for component in point {
                    hash.update(component.to_le_bytes());
                }
            }
            hash.update(brightness.to_le_bytes());
        }
        for component in [
            0xbeffffff_u32,
            0xbf5db3d7,
            0x3f3504f3,
            0x3f800000,
            0x41200000,
            0x4147ae14,
        ] {
            hash.update(component.to_le_bytes());
        }
        for tile in tiles {
            hash.update(tile);
        }
        for (material, location) in locations {
            hash.update(material.to_le_bytes());
            hash.update(location.to_le_bytes());
        }
        Ok(Self {
            tiles,
            locations,
            digest: hash.finalize().into(),
        })
    }

    pub(super) fn same_source(&self, other: &Self) -> bool {
        self.digest == other.digest
            && self.tiles == other.tiles
            && self.locations == other.locations
    }

    pub(super) const fn digest(&self) -> [u8; 32] {
        self.digest
    }

    pub(super) fn raster(&self) -> IconSprite {
        let mut pixels = vec![0; PIXEL_BYTES];
        for (face, positions, uv, brightness) in FACES {
            let positions = positions.map(project);
            for indices in [[0, 1, 2], [0, 2, 3]] {
                triangle(
                    &mut pixels,
                    self.tiles[face as usize],
                    indices.map(|i| positions[i]),
                    indices.map(|i| uv[i]),
                    brightness,
                );
            }
        }
        IconSprite {
            width: 16,
            height: 16,
            rgba8: Arc::from(pixels),
        }
    }
}

fn project([x, y, z]: [f32; 3]) -> [f32; 2] {
    // Canonical f32 sin/cos results for Rx(3.665191411972046) and
    // Ry(0.7853981852531433). Fixed bits keep offline bytes independent of
    // the host math library while retaining the stated rotation order.
    let (sx, cx) = (f32::from_bits(0xbeffffff), f32::from_bits(0xbf5db3d7));
    let (sy, cy) = (f32::from_bits(0x3f3504f3), f32::from_bits(0x3f3504f3));
    let rotated_x = cy * x + sy * z;
    let rotated_z = -sy * x + cy * z;
    [
        1. + 10. * rotated_x,
        f32::from_bits(0x4147ae14) + 10. * (cx * y - sx * rotated_z),
    ]
}

fn edge(a: [f32; 2], b: [f32; 2], p: [f32; 2]) -> f32 {
    (b[0] - a[0]) * (p[1] - a[1]) - (b[1] - a[1]) * (p[0] - a[0])
}

fn triangle(
    pixels: &mut [u8],
    tile: &[u8],
    mut p: [[f32; 2]; 3],
    mut uv: [[f32; 2]; 3],
    brightness: f32,
) {
    if edge(p[0], p[1], p[2]) < 0. {
        p.swap(1, 2);
        uv.swap(1, 2);
    }
    let area = edge(p[0], p[1], p[2]);
    if !area.is_finite() || area <= 0. {
        return;
    }
    for y in 0..16 {
        for x in 0..16 {
            let sample = [x as f32 + 0.5, y as f32 + 0.5];
            let edges = [
                edge(p[1], p[2], sample),
                edge(p[2], p[0], sample),
                edge(p[0], p[1], sample),
            ];
            let boundaries = [(p[1], p[2]), (p[2], p[0]), (p[0], p[1])];
            if !edges.iter().zip(boundaries).all(|(&e, (a, b))| {
                e > 0. || (e == 0. && (b[1] < a[1] || (b[1] == a[1] && b[0] > a[0])))
            }) {
                continue;
            }
            let coord =
                [0, 1].map(|axis| (0..3).map(|i| edges[i] * uv[i][axis] / area).sum::<f32>());
            let tx = (coord[0] * 16.).floor().clamp(0., 15.) as usize;
            let ty = (coord[1] * 16.).floor().clamp(0., 15.) as usize;
            let source = (ty * 16 + tx) * 4;
            let target = (y * 16 + x) * 4;
            for c in 0..3 {
                pixels[target + c] = (f32::from(tile[source + c]) * brightness)
                    .round()
                    .clamp(0., 255.) as u8;
            }
            pixels[target + 3] = 255;
        }
    }
}
