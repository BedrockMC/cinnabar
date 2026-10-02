use crate::ui::UiRenderBatch;

/// Resolve the entire ordered frame before emitting any batch command.
pub(super) fn resolved_batches<'a>(
    accepted_revision: Option<u64>,
    batches: &'a [UiRenderBatch],
    locations: &'a [crate::UiTextureLocation],
    buckets: &[crate::UiTextureBucket],
) -> Option<impl Iterator<Item = (usize, &'a UiRenderBatch, crate::UiTextureLocation)>> {
    if accepted_revision.is_none()
        || batches.iter().any(|batch| {
            locations
                .get(batch.texture_page as usize)
                .is_none_or(|location| {
                    buckets
                        .get(location.bucket)
                        .is_none_or(|bucket| location.layer >= bucket.layers)
                })
        })
    {
        return None;
    }
    Some(
        batches
            .iter()
            .enumerate()
            .map(move |(index, batch)| (index, batch, locations[batch.texture_page as usize])),
    )
}
