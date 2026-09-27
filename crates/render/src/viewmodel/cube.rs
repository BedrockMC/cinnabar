//! Bounded six-face geometry transport. Pose, lighting and action parity remain unavailable.
use super::*;
use assets::{
    BlockFace, BlockFlags, BlockVisualId, NetworkIdMode, RuntimeAssets, VisualKind, VisualSupport,
};
use sha2::{Digest, Sha256};

impl ViewmodelGeometry {
    /// Builds an ordinary opaque cube from the current validated block carrier.
    /// The existing nearest-only 64x64 transport contains six unmodified tiles.
    pub fn opaque_cube(
        assets: &RuntimeAssets,
        visual: BlockVisualId,
    ) -> Option<(Self, ViewmodelSkin)> {
        if !assets.provenance().is_complete() || visual.0 as usize >= assets.visual_count() {
            return None;
        }
        let block = assets.resolve(NetworkIdMode::Sequential, visual.0);
        if !block.is_known()
            || block.kind() != VisualKind::Cube
            || block.support() != VisualSupport::Exact
            || block.flags() != (BlockFlags::CUBE_GEOMETRY | BlockFlags::OCCLUDES_FULL_FACE)
            || block.model_template().is_some()
            || block.animation().is_some()
        {
            return None;
        }
        // Admit the whole source before allocating or copying any pixels.
        let mut tiles = [&[][..]; 6];
        let mut materials = [0; 6];
        for (index, face) in BlockFace::ALL.into_iter().enumerate() {
            let id = block.face(face).material_id();
            if id == assets::DIAGNOSTIC_MATERIAL {
                return None;
            }
            let material = assets.materials().get(id as usize)?;
            if material.flags != 0 || material.animation != assets::NO_ANIMATION {
                return None;
            }
            let page = assets
                .texture_pages()
                .get(material.texture.page() as usize)?;
            let mip = page.texture.mips.first()?;
            if mip.size != 16 || material.texture.layer() >= page.texture.layers {
                return None;
            }
            let first = (material.texture.layer() as usize).checked_mul(16 * 16 * 4)?;
            let tile = mip.rgba8.get(first..first.checked_add(16 * 16 * 4)?)?;
            if !tile.chunks_exact(4).all(|pixel| pixel[3] == 255) {
                return None;
            }
            tiles[index] = tile;
            materials[index] = id;
        }
        let mut pixels = vec![0; 64 * 64 * 4];
        for (index, tile) in tiles.into_iter().enumerate() {
            let [x, y] = [(index % 3) * 16, (index / 3) * 16];
            for row in 0..16 {
                let target = ((y + row) * 64 + x) * 4;
                pixels[target..target + 64].copy_from_slice(&tile[row * 64..row * 64 + 64]);
            }
        }
        // Same face corners and UV directions as the existing world cube pipeline.
        let corners = [
            [[0., 0., 0.], [0., 0., 1.], [0., 1., 1.], [0., 1., 0.]],
            [[1., 0., 0.], [1., 1., 0.], [1., 1., 1.], [1., 0., 1.]],
            [[0., 0., 0.], [1., 0., 0.], [1., 0., 1.], [0., 0., 1.]],
            [[0., 1., 0.], [0., 1., 1.], [1., 1., 1.], [1., 1., 0.]],
            [[0., 0., 0.], [0., 1., 0.], [1., 1., 0.], [1., 0., 0.]],
            [[0., 0., 1.], [1., 0., 1.], [1., 1., 1.], [0., 1., 1.]],
        ];
        let horizontal = [[0., 0.], [1., 0.], [1., 1.], [0., 1.]];
        let transposed = [[0., 0.], [0., 1.], [1., 1.], [1., 0.]];
        let vertical = [[0., 1.], [1., 1.], [1., 0.], [0., 0.]];
        let vertical_transposed = [[0., 1.], [0., 0.], [1., 0.], [1., 1.]];
        let transform = Mat4::from_translation(Vec3::new(0.56, -0.52, -0.72))
            * Mat4::from_rotation_y(45_f32.to_radians())
            * Mat4::from_scale(Vec3::splat(0.4));
        let mut vertices = Vec::with_capacity(36);
        for (face, positions) in corners.into_iter().enumerate() {
            let uv = match face {
                0 | 5 => vertical,
                1 | 4 => vertical_transposed,
                3 => transposed,
                _ => horizontal,
            };
            for corner in [0, 1, 2, 0, 2, 3] {
                let p = transform
                    .transform_point3(Vec3::from_array(positions[corner]) - Vec3::splat(0.5));
                vertices.push(HandVertex {
                    position: p.to_array(),
                    uv: [
                        ((face % 3) as f32 * 16. + 0.5 + uv[corner][0] * 15.) / 64.,
                        ((face / 3) as f32 * 16. + 0.5 + uv[corner][1] * 15.) / 64.,
                    ],
                });
            }
        }
        let pixel_identity: [u8; 32] = Sha256::digest(&pixels).into();
        let mut digest = Sha256::new();
        digest.update(b"opaque-cube-static-v1");
        digest.update(assets.provenance().source_manifest_sha256);
        digest.update(assets.provenance().block_registry_sha256);
        digest.update(visual.0.to_le_bytes());
        for material in materials {
            digest.update(material.to_le_bytes());
        }
        digest.update(pixel_identity);
        digest.update(bytemuck::cast_slice(&vertices));
        let geometry = Self {
            vertices: vertices.into(),
            identity: digest.finalize().into(),
            allowed_rigs: Arc::from([]),
            cube_origin: true,
        };
        let skin = ViewmodelSkin::new(pixels.into(), pixel_identity)?;
        Some((geometry, skin))
    }
}
