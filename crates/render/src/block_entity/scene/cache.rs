//! Static mesh fragments kept in submission order with the original vertex budgets.

use super::{BlockEntitySubmission, BlockEntityVertex, MeshBuilder};

#[derive(Debug)]
pub(super) struct CachedSubmission {
    submission: BlockEntitySubmission,
    start: [usize; 4],
    solid: Vec<BlockEntityVertex>,
    overlay: Vec<BlockEntityVertex>,
    crack: Vec<BlockEntityVertex>,
    additive: Vec<BlockEntityVertex>,
    rejected_quads: u64,
}

/// Counts the vertices already accepted in each draw layer before a submission.
pub(super) fn vertex_counts(builder: &MeshBuilder) -> [usize; 4] {
    [
        builder.solid.len(),
        builder.overlay.len(),
        builder.crack.len(),
        builder.additive.len(),
    ]
}

impl CachedSubmission {
    /// Copies only the vertices and rejected quads produced by this submission.
    pub(super) fn capture(
        submission: &BlockEntitySubmission,
        start: [usize; 4],
        rejected_before: u64,
        builder: &MeshBuilder,
    ) -> Self {
        Self {
            submission: submission.clone(),
            start,
            solid: builder.solid[start[0]..].to_vec(),
            overlay: builder.overlay[start[1]..].to_vec(),
            crack: builder.crack[start[2]..].to_vec(),
            additive: builder.additive[start[3]..].to_vec(),
            rejected_quads: builder.rejected_quads.saturating_sub(rejected_before),
        }
    }

    /// Reuses a fragment only with the same model, light, position and remaining layer budgets.
    pub(super) fn matches(
        &self,
        submission: &BlockEntitySubmission,
        builder: &MeshBuilder,
    ) -> bool {
        &self.submission == submission && self.start == vertex_counts(builder)
    }

    /// Appends the previously accepted geometry at its original position in every draw layer.
    pub(super) fn append_to(&self, builder: &mut MeshBuilder) {
        builder.solid.extend_from_slice(&self.solid);
        builder.overlay.extend_from_slice(&self.overlay);
        builder.crack.extend_from_slice(&self.crack);
        builder.additive.extend_from_slice(&self.additive);
        builder.rejected_quads = builder.rejected_quads.saturating_add(self.rejected_quads);
    }
}
