use protocol::{ActorHandedness, ActorKind};

use super::{ActorSnapshot, ActorStore};
use crate::item::CanonicalItemStack;

/// Most stacked copies drawn for one dropped stack.
pub const MAX_DROPPED_ITEM_COPIES: usize = 5;

// Provisional presentation constants; every value needs independent measurement against the
// 26.30 client before a visual gate closes.
const CENTER_HEIGHT: f32 = 0.125;
const BOB_AMPLITUDE: f32 = 0.1;
const BOB_RATE_PER_TICK: f32 = 0.1;
const SPIN_RATE_PER_TICK: f32 = 0.05;
const COPY_SPREAD_XZ: f32 = 0.15;
const COPY_SPREAD_Y: f32 = 0.05;
const DEFAULT_COLLECTOR_HEIGHT: f32 = 1.8;

/// One dropped-item stack ready to draw: interpolated origin, spin, and stacked copy offsets.
#[derive(Debug, Clone, PartialEq)]
pub struct DroppedItemView {
    pub runtime_id: u64,
    /// World position of the item's visual centre before per-copy offsets.
    pub position: [f32; 3],
    pub yaw_radians: f32,
    pub item: CanonicalItemStack,
    /// Per-copy offsets from `position`; only the first `copy_count` entries are live.
    pub copy_offsets: [[f32; 3]; MAX_DROPPED_ITEM_COPIES],
    pub copy_count: u8,
}

/// Visible copies for a stack of `count` items.
#[must_use]
pub const fn dropped_item_copy_count(count: u16) -> u8 {
    match count {
        0 | 1 => 1,
        2..=16 => 2,
        17..=32 => 3,
        33..=48 => 4,
        _ => 5,
    }
}

/// Stable pseudo-random unit values in `-1..=1` for one actor and copy index.
fn jitter(runtime_id: u64, index: u64, salt: u64) -> f32 {
    let mut state = runtime_id
        .wrapping_mul(0x9E37_79B9_7F4A_7C15)
        .wrapping_add(index.wrapping_mul(0xBF58_476D_1CE4_E5B9))
        .wrapping_add(salt.wrapping_mul(0x94D0_49BB_1331_11EB));
    state ^= state >> 30;
    state = state.wrapping_mul(0xBF58_476D_1CE4_E5B9);
    state ^= state >> 27;
    ((state >> 40) as f32 / (1_u64 << 23) as f32) - 1.0
}

/// Per-actor phase so neighbouring drops do not bob and spin in lockstep.
fn phase(runtime_id: u64) -> f32 {
    (jitter(runtime_id, 0, 7) + 1.0) * std::f32::consts::PI
}

fn copy_offsets(runtime_id: u64, count: u8) -> [[f32; 3]; MAX_DROPPED_ITEM_COPIES] {
    let mut offsets = [[0.0; 3]; MAX_DROPPED_ITEM_COPIES];
    // The first copy stays on the entity origin.
    for (index, offset) in offsets
        .iter_mut()
        .enumerate()
        .take(usize::from(count))
        .skip(1)
    {
        let index = index as u64;
        *offset = [
            jitter(runtime_id, index, 1) * COPY_SPREAD_XZ,
            jitter(runtime_id, index, 2) * COPY_SPREAD_Y,
            jitter(runtime_id, index, 3) * COPY_SPREAD_XZ,
        ];
    }
    offsets
}

fn interpolated(actor: &ActorSnapshot, alpha: f32) -> [f32; 3] {
    std::array::from_fn(|axis| {
        actor.previous_pose.position[axis]
            + (actor.position[axis] - actor.previous_pose.position[axis]) * alpha
    })
}

impl ActorStore {
    /// Views for every dropped-item actor whose stack resolved; picked-up items fly to their
    /// collector and vanish once they arrive.
    pub(crate) fn dropped_items(&self, partial_tick: f32) -> Vec<DroppedItemView> {
        let alpha = partial_tick.clamp(0.0, 1.0);
        let mut views = self
            .actors
            .values()
            .filter_map(|actor| self.dropped_item_view(actor, alpha))
            .collect::<Vec<_>>();
        views.sort_unstable_by_key(|view| view.runtime_id);
        views
    }

    fn dropped_item_view(&self, actor: &ActorSnapshot, alpha: f32) -> Option<DroppedItemView> {
        let ActorKind::Entity { identifier } = &actor.kind else {
            return None;
        };
        if identifier.as_ref() != "minecraft:item" {
            return None;
        }
        let item = self
            .equipment_in_hand(actor.runtime_id, ActorHandedness::Right)?
            .item
            .clone();
        if item.identity.is_empty() {
            return None;
        }
        let ticks = actor.status.age_ticks as f32 + alpha;
        let phase = phase(actor.runtime_id);
        let bob = ((ticks * BOB_RATE_PER_TICK + phase).sin() + 1.0) * BOB_AMPLITUDE;
        let feet = interpolated(actor, alpha);
        let mut position = [feet[0], feet[1] + CENTER_HEIGHT + bob, feet[2]];
        if let Some(pickup) = actor.status.pickup {
            let progress =
                (f32::from(pickup.ticks) + alpha) / f32::from(super::PICKUP_DURATION_TICKS);
            if progress >= 1.0 {
                return None;
            }
            if let Some(collector) = self.actors.get(&pickup.collector_runtime_id) {
                let height = collector
                    .bounding_box()
                    .map_or(DEFAULT_COLLECTOR_HEIGHT, |(min, max)| max[1] - min[1]);
                let anchor = interpolated(collector, alpha);
                let target = [anchor[0], anchor[1] + height * 0.5, anchor[2]];
                position = std::array::from_fn(|axis| {
                    position[axis] + (target[axis] - position[axis]) * progress
                });
            }
        }
        let copy_count = dropped_item_copy_count(item.identity.count);
        Some(DroppedItemView {
            runtime_id: actor.runtime_id,
            position,
            yaw_radians: ticks * SPIN_RATE_PER_TICK + phase,
            item,
            copy_offsets: copy_offsets(actor.runtime_id, copy_count),
            copy_count,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn copy_tiers_grow_with_the_stack() {
        assert_eq!(dropped_item_copy_count(1), 1);
        assert_eq!(dropped_item_copy_count(2), 2);
        assert_eq!(dropped_item_copy_count(17), 3);
        assert_eq!(dropped_item_copy_count(40), 4);
        assert_eq!(dropped_item_copy_count(64), 5);
        assert_eq!(
            dropped_item_copy_count(u16::MAX),
            MAX_DROPPED_ITEM_COPIES as u8
        );
    }

    #[test]
    fn copy_offsets_are_stable_bounded_and_first_copy_is_centred() {
        let a = copy_offsets(42, 5);
        assert_eq!(a, copy_offsets(42, 5));
        assert_eq!(a[0], [0.0; 3]);
        for offset in &a[1..] {
            assert!(offset[0].abs() <= COPY_SPREAD_XZ && offset[2].abs() <= COPY_SPREAD_XZ);
            assert!(offset[1].abs() <= COPY_SPREAD_Y);
        }
        assert_ne!(a[1], copy_offsets(43, 5)[1]);
        assert_eq!(copy_offsets(42, 1)[1], [0.0; 3]);
    }

    #[test]
    fn jitter_stays_within_unit_range() {
        for id in 0..64 {
            let value = jitter(id, id + 1, 3);
            assert!((-1.0..=1.0).contains(&value));
        }
    }
}
