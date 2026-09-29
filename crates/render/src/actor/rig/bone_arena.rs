//! Bone matrices appended directly to the frame arena without per-actor temporary buffers.

use super::{RenderBoneTransform, affine_matrix};

/// Appends one pose to its final arena; an invalid bone restores the original length.
pub(super) fn append_pose_matrices(
    arena: &mut Vec<[[f32; 4]; 3]>,
    transforms: &[RenderBoneTransform],
    pivots: &[[f32; 3]],
) -> bool {
    let start = arena.len();
    arena.reserve(transforms.len());
    for (index, transform) in transforms.iter().enumerate() {
        let Some(matrix) =
            affine_matrix(*transform, pivots.get(index).copied().unwrap_or([0.0; 3]))
        else {
            arena.truncate(start);
            return false;
        };
        arena.push(matrix);
    }
    true
}

#[cfg(test)]
#[path = "bone_arena/tests.rs"]
mod tests;
