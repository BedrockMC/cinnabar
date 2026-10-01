use super::model::PreparedBlockMutations;
use super::*;

impl WorldStream {
    pub(super) fn accept_decode_completion(&mut self, completion: DecodeCompletion) {
        self.stats.phase2_stages.decode_jobs_completed = self
            .stats
            .phase2_stages
            .decode_jobs_completed
            .saturating_add(1);
        self.in_flight_decode_jobs = self.in_flight_decode_jobs.saturating_sub(1);
        self.stats.observe_decode_queue_wait(completion.queue_wait);
        if self.blocking_block_updates == Some(completion.sequence)
            && matches!(&completion.event, PreparedWorldEvent::BlockUpdates { .. })
        {
            self.blocking_block_updates = None;
            self.submitted.remove(&completion.sequence);
            self.heavy_sequences.remove(&completion.sequence);
            self.apply_prepared(completion.event);
            self.reapply_deferred_predictions();
            if !self.polling {
                self.apply_ready();
            }
            return;
        }
        if self
            .ordered
            .insert(completion.sequence, completion.event)
            .is_err()
        {
            self.heavy_sequences.remove(&completion.sequence);
            self.record_normalization_error(NormalizationErrorReason::OrderedCompletionRejection);
        }
    }
    pub(super) fn snapshot_block_mutation_batches(
        &mut self,
        events: Vec<BlockUpdateEvent>,
    ) -> Vec<BlockMutationBatch> {
        let mut grouped = BTreeMap::<SubChunkKey, Vec<BlockUpdate>>::new();
        for event in events {
            match split_block_update(event) {
                Ok((key, update)) if self.column_is_data_interesting(key.chunk()) => {
                    grouped.entry(key).or_default().push(update);
                }
                Ok(_) => {
                    self.record_normalization_error(NormalizationErrorReason::InactiveBlockUpdate)
                }
                Err(_) => {
                    self.record_normalization_error(NormalizationErrorReason::MalformedBlockUpdate)
                }
            }
        }
        grouped
            .into_iter()
            .map(|(key, updates)| BlockMutationBatch {
                key,
                previous: self.store.sub_chunk(key),
                updates,
            })
            .collect()
    }
    pub(super) fn dispatch_decode_jobs(&mut self) {
        let budget = DECODE_DISPATCH_BUDGET_PER_POLL
            .min(MAX_IN_FLIGHT_DECODE_JOBS.saturating_sub(self.in_flight_decode_jobs));
        for index in 0..budget {
            if index != 0 && self.poll_budget_exhausted() {
                break;
            }
            let Some(QueuedDecodeJob { queued_at, job }) = self.pending_decode.pop_front() else {
                break;
            };
            self.in_flight_decode_jobs += 1;
            self.stats.phase2_stages.decode_jobs_dispatched = self
                .stats
                .phase2_stages
                .decode_jobs_dispatched
                .saturating_add(1);
            let tx = self.decode_tx.clone();
            rayon::spawn(move || {
                let started = Instant::now();
                let queue_wait = queue_wait(queued_at, started);
                let completion = match job {
                    DecodeJob::InlineLevelChunk {
                        sequence,
                        event,
                        payload,
                        slots,
                        count,
                        ids,
                    } => {
                        let chunk = ChunkKey::new(event.dimension, event.x, event.z);
                        let decoded = DecodedLevelChunk::decode_inline(
                            chunk, slots, count, &payload, &ids, &ids,
                        );
                        DecodeCompletion {
                            sequence,
                            queue_wait,
                            event: PreparedWorldEvent::InlineLevelChunk {
                                event,
                                decoded,
                                duration: started.elapsed(),
                            },
                        }
                    }
                    DecodeJob::RequestLevelChunk {
                        sequence,
                        event,
                        payload,
                        slots,
                        ids,
                    } => {
                        let chunk = ChunkKey::new(event.dimension, event.x, event.z);
                        let decoded = decode_column_tail(chunk, slots, &payload, &ids);
                        DecodeCompletion {
                            sequence,
                            queue_wait,
                            event: PreparedWorldEvent::RequestLevelChunk {
                                event,
                                decoded,
                                duration: started.elapsed(),
                            },
                        }
                    }
                    DecodeJob::SubChunks {
                        sequence,
                        batch,
                        ids,
                    } => {
                        let dimension = batch.dimension;
                        let entries = prepare_sub_chunks(batch, &ids);
                        DecodeCompletion {
                            sequence,
                            queue_wait,
                            event: PreparedWorldEvent::SubChunks {
                                dimension,
                                entries,
                                duration: started.elapsed(),
                            },
                        }
                    }
                    DecodeJob::BlockUpdates {
                        sequence,
                        batches,
                        ids,
                    } => {
                        let result = prepare_block_mutations(batches, &ids);
                        DecodeCompletion {
                            sequence,
                            queue_wait,
                            event: PreparedWorldEvent::BlockUpdates {
                                result,
                                duration: started.elapsed(),
                            },
                        }
                    }
                    DecodeJob::BlockEntityUpdate { sequence, event } => {
                        let key = BlockEntityKey::new(
                            event.dimension,
                            event.position[0],
                            event.position[1],
                            event.position[2],
                        );
                        let decoded = DecodedBlockEntities::decode_live(key, &event.nbt);
                        DecodeCompletion {
                            sequence,
                            queue_wait,
                            event: PreparedWorldEvent::BlockEntityUpdate {
                                key,
                                decoded,
                                duration: started.elapsed(),
                            },
                        }
                    }
                };
                let _ = tx.send(completion);
            });
        }
    }
}

/// Prepares packed mutations and the same light comparison used by direct predictions.
pub(super) fn prepare_block_mutations(
    batches: Vec<BlockMutationBatch>,
    ids: &DecodeIds,
) -> Result<PreparedBlockMutations, MutationError> {
    let mut prepared = PreparedBlockMutations {
        mutations: Vec::with_capacity(batches.len()),
        relight: BTreeSet::new(),
    };
    for mut batch in batches {
        for update in &mut batch.updates {
            update.runtime_id = BlockIds::resolve(ids, update.runtime_id);
        }
        let mutation = ChunkStore::prepare_sub_chunk_blocks(
            batch.key,
            batch.previous.as_deref(),
            &batch.updates,
            ids.air(),
        )?;
        if mutation.changed()
            && WorldStream::light_semantics_changed(
                BlockClassifier::new(ids.air),
                &ids.assets,
                ids.mode,
                batch.previous.as_deref(),
                mutation.replacement(),
            )
        {
            prepared.relight.insert(mutation.key());
        }
        prepared.mutations.push(mutation);
    }
    Ok(prepared)
}

/// Session registries that decode workers resolve raw ids against, as the
/// vanilla palettes do: unknown blocks become air and unknown biomes the
/// dimension's fallback biome.
#[derive(Clone)]
pub(super) struct DecodeIds {
    pub(super) assets: Arc<RuntimeAssets>,
    pub(super) custom_blocks: std::ops::Range<u32>,
    pub(super) remap: Arc<assets::SequentialIdRemap>,
    pub(super) mode: NetworkIdMode,
    pub(super) air: u32,
    pub(super) biome_tints: Arc<ResolvedBiomeTints>,
    pub(super) default_biome: u32,
}

impl std::fmt::Debug for DecodeIds {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("DecodeIds")
            .field("mode", &self.mode)
            .field("air", &self.air)
            .field("default_biome", &self.default_biome)
            .finish_non_exhaustive()
    }
}

impl BlockIds for DecodeIds {
    fn air(&self) -> u32 {
        self.air
    }

    fn resolve(&self, network_id: u32) -> u32 {
        let network_id = if self.mode == NetworkIdMode::Sequential {
            self.remap.to_internal(network_id)
        } else {
            network_id
        };
        if self.assets.is_known(self.mode, network_id) || self.custom_blocks.contains(&network_id) {
            network_id
        } else {
            self.air
        }
    }
}

impl BiomeIds for DecodeIds {
    fn default_biome(&self) -> u32 {
        self.default_biome
    }

    fn resolve(&self, biome_id: u16) -> u32 {
        let biome_id = u32::from(biome_id);
        if !self.assets.is_diagnostic()
            && self.biome_tints.dense_index(biome_id) == assets::MISSING_BIOME_DENSE_INDEX
        {
            self.default_biome
        } else {
            biome_id
        }
    }
}

impl WorldStream {
    /// Sequential ids of this session's server-defined blocks, which decode as known.
    pub fn set_custom_block_ids(&mut self, ids: std::ops::Range<u32>) {
        self.custom_block_ids = ids;
    }

    /// Translates sequential wire ids when custom blocks sort among vanilla names.
    pub fn set_sequential_id_remap(&mut self, remap: assets::SequentialIdRemap) {
        self.id_remap = Arc::new(remap);
    }

    pub(super) fn decode_ids(&self, dimension: i32) -> DecodeIds {
        DecodeIds {
            assets: Arc::clone(&self.runtime_assets),
            custom_blocks: self.custom_block_ids.clone(),
            remap: Arc::clone(&self.id_remap),
            mode: self.network_id_mode,
            air: self.classifier.air_network_id(),
            biome_tints: Arc::clone(&self.resolved_biome_tints),
            default_biome: default_biome_id(dimension),
        }
    }
}

pub(super) fn dimension_slots(range: DimensionRange) -> DimensionSlots {
    DimensionSlots {
        base_sub_chunk_y: range.base_sub_chunk_y,
        count: range.sub_chunk_count,
    }
}

/// Vanilla's fallback biome ids: ocean, hell, and the_end.
pub(super) fn default_biome_id(dimension: i32) -> u32 {
    match dimension {
        1 => 8,
        2 => 9,
        _ => 0,
    }
}
