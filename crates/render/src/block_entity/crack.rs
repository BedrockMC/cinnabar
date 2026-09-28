//! Break-progress overlay: the destroy-stage texture over each face of the block.

use super::{
    atlas::BlockEntityAtlas,
    mesh::{Layer, MeshBuilder, WHITE},
    scene::CrackInstance,
};

/// Outward push that keeps the overlay in front of the block's own faces.
const FACE_OFFSET: f32 = 0.002;

/// Stages are `0..=9`; larger values clamp to the last.
#[must_use]
pub fn crack_texture_name(stage: u8) -> String {
    format!("textures/environment/destroy_stage_{}", stage.min(9))
}

pub(super) fn emit_crack(
    builder: &mut MeshBuilder,
    atlas: &BlockEntityAtlas,
    crack: CrackInstance,
) {
    let Some(texture) = atlas.texture(&crack_texture_name(crack.stage), [16.0, 16.0]) else {
        return;
    };
    let [bx, by, bz] = crack.block.map(|value| value as f32);
    let (x0, x1) = (bx - FACE_OFFSET, bx + 1.0 + FACE_OFFSET);
    let (y0, y1) = (by - FACE_OFFSET, by + 1.0 + FACE_OFFSET);
    let (z0, z1) = (bz - FACE_OFFSET, bz + 1.0 + FACE_OFFSET);
    let faces: [[[f32; 3]; 4]; 6] = [
        [[x1, y1, z0], [x0, y1, z0], [x0, y0, z0], [x1, y0, z0]],
        [[x0, y1, z1], [x1, y1, z1], [x1, y0, z1], [x0, y0, z1]],
        [[x1, y1, z1], [x1, y1, z0], [x1, y0, z0], [x1, y0, z1]],
        [[x0, y1, z0], [x0, y1, z1], [x0, y0, z1], [x0, y0, z0]],
        [[x1, y1, z1], [x0, y1, z1], [x0, y1, z0], [x1, y1, z0]],
        [[x1, y0, z0], [x0, y0, z0], [x0, y0, z1], [x1, y0, z1]],
    ];
    for corners in faces {
        builder.textured_quad(Layer::Overlay, corners, texture.rect, WHITE);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stage_names_clamp_to_the_last_texture() {
        assert_eq!(
            crack_texture_name(3),
            "textures/environment/destroy_stage_3"
        );
        assert_eq!(
            crack_texture_name(200),
            "textures/environment/destroy_stage_9"
        );
    }
}
