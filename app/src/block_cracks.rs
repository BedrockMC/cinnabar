//! Bounded consumption of server-authored cracking state, not a timing model.
//!
//! Values remain opaque validated server values. This reducer does not infer
//! progress, atlas stages, expiry, local destruction, or rendering behavior.

use std::collections::BTreeMap;

use protocol::{BlockCrackAction, BlockCrackEvent};
use world::ChunkCollisionRevision;

use crate::ui_runtime::{SequencedBlockCrackEvent, UiRuntime, UiRuntimeError};

pub(crate) fn consume_committed_block_crack(
    ui: &mut UiRuntime,
    session_id: u64,
    sequence: u64,
    dimension: i32,
    event: BlockCrackEvent,
) -> Result<(), UiRuntimeError> {
    ui.retain_block_crack(SequencedBlockCrackEvent {
        session_id,
        fifo_sequence: sequence,
        dimension,
        event,
    })
}

pub(crate) fn reconcile_world_block_cracks(
    ui: &mut UiRuntime,
    stream: &client_world::WorldStream,
    assets: &assets::RuntimeAssets,
) {
    ui.reconcile_block_cracks(|position| {
        sample_target(
            stream.collision_store(),
            stream.current_dimension(),
            position,
            stream.network_id_mode(),
            assets,
        )
    });
}

fn sample_target(
    store: &world::ChunkStore,
    dimension: i32,
    position: [i32; 3],
    mode: assets::NetworkIdMode,
    assets: &assets::RuntimeAssets,
) -> Option<CrackTargetIdentity> {
    let [x, y, z] = position;
    let key = world::SubChunkKey::new(
        dimension,
        x.div_euclid(16),
        y.div_euclid(16),
        z.div_euclid(16),
    );
    if !store.is_sub_chunk_loaded(key) {
        return None;
    }
    let sub_chunk = store.sub_chunk(key)?;
    let [x, y, z] =
        [x, y, z].map(|value| u8::try_from(value.rem_euclid(16)).expect("local coordinate"));
    let runtime_id = sub_chunk.runtime_id(0, x, y, z)?;
    let block = assets.resolve(mode, runtime_id);
    if !block.is_known() || block.flags().contains(assets::BlockFlags::AIR) {
        return None;
    }
    let mut layers = [None; world::MAX_STORAGE_COUNT];
    for (layer, value) in layers.iter_mut().enumerate() {
        *value = sub_chunk.runtime_id(layer, x, y, z);
    }
    Some(CrackTargetIdentity {
        runtime_id,
        layers,
        column: store.collision_revision(key.chunk())?,
    })
}

/// Client presentation budget, not a vanilla gameplay limit.
pub(crate) const MAX_ACTIVE_BLOCK_CRACKS: usize = 1_024;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct CrackTargetIdentity {
    pub(crate) runtime_id: u32,
    pub(crate) layers: [Option<u32>; world::MAX_STORAGE_COUNT],
    pub(crate) column: ChunkCollisionRevision,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ActiveCrack {
    server_value: u16,
    target: Option<CrackTargetIdentity>,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct BlockCrackStatus {
    pub(crate) active: usize,
    /// Diagnostic aggregate of retained values, not accumulated progress.
    pub(crate) server_value_sum: u64,
    pub(crate) consumed: u64,
    pub(crate) orphan_updates: u64,
    pub(crate) capacity_rejections: u64,
    pub(crate) unsupported_values: u64,
    pub(crate) wrong_dimension: u64,
    pub(crate) retired_targets: u64,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct BlockCracks {
    dimension: Option<i32>,
    active: BTreeMap<[i32; 3], ActiveCrack>,
    status: BlockCrackStatus,
    reported_status: Option<BlockCrackStatus>,
}

impl BlockCracks {
    pub(crate) fn status(&self) -> BlockCrackStatus {
        BlockCrackStatus {
            active: self.active.len(),
            server_value_sum: self
                .active
                .values()
                .map(|entry| u64::from(entry.server_value))
                .sum(),
            ..self.status
        }
    }

    /// Replacement clears active state but does not reset the caller's FIFO
    /// watermark. Only a session replacement may reset that watermark.
    pub(crate) fn synchronize_dimension(&mut self, dimension: Option<i32>) {
        if self.dimension != dimension {
            self.active.clear();
            self.dimension = dimension;
        }
    }

    pub(crate) fn consume(&mut self, dimension: i32, event: BlockCrackEvent) {
        self.status.consumed = self.status.consumed.saturating_add(1);
        match self.dimension {
            None => self.dimension = Some(dimension),
            Some(current) if current != dimension => {
                self.status.wrong_dimension = self.status.wrong_dimension.saturating_add(1);
                return;
            }
            Some(_) => {}
        }
        match event.action {
            BlockCrackAction::Stop => {
                self.active.remove(&event.position);
            }
            BlockCrackAction::Start {
                progress_per_tick: 0,
            }
            | BlockCrackAction::UpdateSpeed {
                progress_per_tick: 0,
            } => {
                // Normalized wire ingress already excludes zero. Keep direct
                // typed ingress lenient too; do not corrupt an existing key.
                self.status.unsupported_values = self.status.unsupported_values.saturating_add(1);
            }
            BlockCrackAction::Start { progress_per_tick } => {
                if !self.active.contains_key(&event.position)
                    && self.active.len() >= MAX_ACTIVE_BLOCK_CRACKS
                {
                    self.status.capacity_rejections =
                        self.status.capacity_rejections.saturating_add(1);
                    if self.status.capacity_rejections.is_power_of_two() {
                        bevy::log::warn!(
                            target: "bedrock_client::block_cracks",
                            active = self.active.len(),
                            rejected = self.status.capacity_rejections,
                            "block crack presentation capacity exhausted"
                        );
                    }
                    return;
                }
                self.active.insert(
                    event.position,
                    ActiveCrack {
                        server_value: progress_per_tick,
                        target: None,
                    },
                );
            }
            BlockCrackAction::UpdateSpeed { progress_per_tick } => {
                if let Some(active) = self.active.get_mut(&event.position) {
                    active.server_value = progress_per_tick;
                } else {
                    self.status.orphan_updates = self.status.orphan_updates.saturating_add(1);
                }
            }
        }
    }

    /// Requery the exact cell and all bounded layers. An unrelated column
    /// mutation must not retire unchanged cell state. Unknown/air/unloaded
    /// targets are not presentation authority. This snapshot seam cannot
    /// distinguish same-state replacement entirely between observations.
    pub(crate) fn reconcile_targets(
        &mut self,
        mut target_at: impl FnMut([i32; 3]) -> Option<CrackTargetIdentity>,
    ) {
        let before = self.active.len();
        self.active.retain(|position, active| {
            let Some(current) = target_at(*position) else {
                return false;
            };
            if active.target.is_some_and(|previous| {
                previous.runtime_id != current.runtime_id
                    || previous.layers != current.layers
                    || previous.column.chunk != current.column.chunk
            }) {
                return false;
            }
            active.target = Some(current);
            true
        });
        self.status.retired_targets = self
            .status
            .retired_targets
            .saturating_add((before - self.active.len()) as u64);
    }

    pub(crate) fn report_status(&mut self, session_id: u64, status: BlockCrackStatus) {
        if self.reported_status == Some(status) {
            return;
        }
        self.reported_status = Some(status);
        bevy::log::debug!(
            target: "bedrock_client::block_cracks",
            session_id,
            dimension = ?self.dimension,
            active = status.active,
            server_value_sum = status.server_value_sum,
            consumed = status.consumed,
            orphan_updates = status.orphan_updates,
            capacity_rejections = status.capacity_rejections,
            unsupported_values = status.unsupported_values,
            wrong_dimension = status.wrong_dimension,
            retired_targets = status.retired_targets,
            "block crack state consumed"
        );
    }
}

#[cfg(test)]
mod tests;
