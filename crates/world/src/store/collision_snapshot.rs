use super::*;

impl ChunkStore {
    /// Retains immutable block palettes, availability and revisions for prediction replay.
    pub fn collision_snapshot(&self) -> Self {
        Self {
            chunks: self
                .chunks
                .iter()
                .map(|(&key, chunk)| {
                    (
                        key,
                        Chunk {
                            sub_chunks: chunk.sub_chunks.clone(),
                            ..Chunk::default()
                        },
                    )
                })
                .collect(),
            loaded_chunks: self.loaded_chunks.clone(),
            authoritative_sub_chunks: self.authoritative_sub_chunks.clone(),
            collision_revisions: self.collision_revisions.clone(),
            collision_revision_allocator: Arc::clone(&self.collision_revision_allocator),
        }
    }
}
