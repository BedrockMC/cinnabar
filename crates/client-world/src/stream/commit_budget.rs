use super::*;

/// Cooperative frame allocation, with one progress item per ready service lane.
pub(super) const WORLD_POLL_BUDGET: Duration = Duration::from_millis(2);

pub(super) struct PendingSubChunkCommit {
    pub(super) sequence: u64,
    dimension: i32,
    entries: std::vec::IntoIter<PreparedSubChunk>,
    duration: Duration,
}

impl WorldStream {
    /// Starts the frame's shared ingress, commit and scheduling allocation.
    pub fn begin_frame_work(&mut self) {
        self.poll_deadline = Some(Instant::now() + WORLD_POLL_BUDGET);
    }

    /// Reports whether normal work has spent this poll's shared allocation.
    pub(super) fn poll_budget_exhausted(&self) -> bool {
        self.poll_deadline
            .is_some_and(|deadline| Instant::now() >= deadline)
    }

    /// Commits FIFO work without crossing a partially published batch or mutation fence.
    pub(super) fn apply_ready(&mut self) {
        if self.blocking_block_updates.is_some() {
            return;
        }
        let deadline = self
            .poll_deadline
            .unwrap_or_else(|| Instant::now() + WORLD_POLL_BUDGET);
        let mut progressed = self.poll_deadline.is_some() && !self.polling;
        while !progressed || Instant::now() < deadline {
            if let Some(mut pending) = self.pending_sub_chunk_commit.take() {
                if let Some(entry) = pending.entries.next() {
                    self.apply_prepared_with_sequence(
                        PreparedWorldEvent::SubChunks {
                            dimension: pending.dimension,
                            entries: vec![entry],
                            duration: pending.duration,
                        },
                        Some(pending.sequence),
                    );
                }
                if pending.entries.len() == 0 {
                    self.finish_ordered_commit(pending.sequence);
                } else {
                    self.pending_sub_chunk_commit = Some(pending);
                }
                progressed = true;
                continue;
            }
            let Some(event) = self.ordered.pop_next() else {
                break;
            };
            let sequence = self.ordered.next_sequence().saturating_sub(1);
            match event {
                PreparedWorldEvent::SubChunks {
                    dimension,
                    entries,
                    duration,
                } => {
                    self.pending_sub_chunk_commit = Some(PendingSubChunkCommit {
                        sequence,
                        dimension,
                        entries: entries.into_iter(),
                        duration,
                    });
                    continue;
                }
                PreparedWorldEvent::Immediate(WorldEvent::BlockUpdates(events)) => {
                    let batches = self.snapshot_block_mutation_batches(events);
                    if batches.is_empty() {
                        self.finish_ordered_commit(sequence);
                    } else {
                        let ids = self.decode_ids(self.current_dimension);
                        self.predictions.begin_server_batch();
                        self.enqueue_decode_job(DecodeJob::BlockUpdates {
                            sequence,
                            batches,
                            ids,
                        });
                        self.blocking_block_updates = Some(sequence);
                        break;
                    }
                }
                event => {
                    self.apply_prepared_with_sequence(event, Some(sequence));
                    self.finish_ordered_commit(sequence);
                }
            }
            progressed = true;
        }
    }

    /// Releases admission only after the entire ordered event has committed.
    fn finish_ordered_commit(&mut self, sequence: u64) {
        self.submitted.remove(&sequence);
        self.heavy_sequences.remove(&sequence);
        self.cancel_request_reservation(sequence);
    }
}
