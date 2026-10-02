use super::*;

impl ChunkStore {
    /// Shares unchanged collision indexes; column palette indexes copy only on mutation.
    pub fn collision_snapshot(&self) -> Arc<Self> {
        Arc::clone(self.collision_snapshot.get_or_init(|| {
            Arc::new(Self {
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
                collision_snapshot: std::sync::OnceLock::new(),
            })
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unchanged_snapshots_share_indexes_and_mutations_keep_old_palettes() {
        let mut store = ChunkStore::new();
        let key = SubChunkKey::new(0, 0, 0, 0);
        store.mark_sub_chunk_loaded(key).unwrap();
        store
            .update_block(key, BlockUpdate::new(1, 1, 1, 0, 1), 0)
            .unwrap();
        let first = store.collision_snapshot();
        let second = store.collision_snapshot();
        assert!(std::ptr::eq(
            first.chunk(key.chunk()).unwrap(),
            second.chunk(key.chunk()).unwrap()
        ));
        store
            .update_block(key, BlockUpdate::new(1, 1, 1, 0, 2), 0)
            .unwrap();
        let changed = store.collision_snapshot();
        assert_eq!(
            first.sub_chunk(key).unwrap().runtime_id(0, 1, 1, 1),
            Some(1)
        );
        assert_eq!(
            changed.sub_chunk(key).unwrap().runtime_id(0, 1, 1, 1),
            Some(2)
        );
        assert_ne!(
            first.collision_revision(key.chunk()),
            changed.collision_revision(key.chunk())
        );
        store.evict_chunk(key.chunk());
        assert!(!store.collision_snapshot().is_sub_chunk_loaded(key));
        assert!(first.is_sub_chunk_loaded(key));
    }
}
