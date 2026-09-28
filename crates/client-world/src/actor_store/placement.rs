use protocol::ActorMetadataValue;

use super::ActorStore;

/// Seat offset, mount-local, streamed on the rider (protocol `EntityDataKeySeatOffset`).
const KEY_SEAT_OFFSET: u32 = 56;
const KEY_BED_POSITION: u32 = 28;

/// World offset of a mount-local seat `[right, up, forward]` for a mount facing `yaw_degrees`.
pub(super) fn seat_world_offset(local: [f32; 3], yaw_degrees: f32) -> [f32; 3] {
    let (sin, cos) = yaw_degrees.to_radians().sin_cos();
    [
        local[0] * cos - local[2] * sin,
        local[1],
        local[0] * sin + local[2] * cos,
    ]
}

/// One rider position of a mount type, usable while its rider count is in `min..=max`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RideSeat {
    /// Mount-local `[right, up, forward]`.
    pub position: [f32; 3],
    pub min_riders: u32,
    pub max_riders: u32,
}

/// Seat layouts by mount identifier, used when the server streams no seat offset.
#[derive(Clone, Debug, Default)]
pub struct SeatDefaults {
    seats: std::collections::HashMap<std::sync::Arc<str>, Vec<RideSeat>>,
}

impl SeatDefaults {
    pub fn insert(&mut self, identifier: impl Into<std::sync::Arc<str>>, seats: Vec<RideSeat>) {
        if !seats.is_empty() {
            self.seats.insert(identifier.into(), seats);
        }
    }

    /// Seat for the `index`th of `riders` riders; an index past the seats takes the last one.
    fn seat(&self, mount: &str, riders: u32, index: usize) -> Option<[f32; 3]> {
        let usable: Vec<_> = self
            .seats
            .get(mount)?
            .iter()
            .filter(|seat| (seat.min_riders..=seat.max_riders).contains(&riders))
            .collect();
        usable
            .get(index)
            .or(usable.last())
            .map(|seat| seat.position)
    }
}

impl ActorStore {
    pub(crate) fn set_seat_defaults(&mut self, defaults: std::sync::Arc<SeatDefaults>) {
        self.seat_defaults = defaults;
    }

    /// Streamed seat offset of a rider, else the mount type's default for its rider index
    /// (riders of one mount are ordered by unique id).
    fn seat_offset(
        &self,
        rider_unique_id: i64,
        rider_id: u64,
        mount: &super::ActorSnapshot,
    ) -> Option<[f32; 3]> {
        if let Some(ActorMetadataValue::Vector(seat)) =
            self.actors.get(&rider_id)?.metadata.get(&KEY_SEAT_OFFSET)
        {
            return seat.iter().all(|axis| axis.is_finite()).then_some(*seat);
        }
        let protocol::ActorKind::Entity { identifier } = &mount.kind else {
            return None;
        };
        let mut riders: Vec<i64> = self
            .rider_to_ridden
            .iter()
            .filter(|(_, ridden)| **ridden == mount.unique_id)
            .map(|(rider, _)| *rider)
            .collect();
        riders.sort_unstable();
        let index = riders.iter().position(|rider| *rider == rider_unique_id)?;
        let name = identifier.as_ref();
        self.seat_defaults.seat(name, riders.len() as u32, index)
    }

    /// Places each linked rider at its seat; riders with no known seat keep their streamed pose.
    /// The local rig is client-fed and skipped.
    pub(super) fn seat_riders(&mut self) {
        let placements: Vec<(u64, [f32; 3])> = self
            .rider_to_ridden
            .iter()
            .filter_map(|(rider, ridden)| {
                let rider_id = *self.unique_to_runtime.get(rider)?;
                if self.remote_state_excluded_runtime_id == Some(rider_id) {
                    return None;
                }
                let mount = self.actors.get(self.unique_to_runtime.get(ridden)?)?;
                let seat = self.seat_offset(*rider, rider_id, mount)?;
                let offset = seat_world_offset(seat, mount.yaw);
                Some((
                    rider_id,
                    std::array::from_fn(|axis| mount.position[axis] + offset[axis]),
                ))
            })
            .collect();
        for (runtime_id, position) in placements {
            if let Some(rider) = self.actors.get_mut(&runtime_id) {
                rider.received_pose.position = position;
                rider.position = position;
                rider.interpolation_ticks_remaining = 0;
            }
        }
    }

    /// Feet position of every tracked actor, for the caller's fluid sampling.
    pub(crate) fn fluid_sample_points(&self) -> Vec<(u64, [f32; 3])> {
        self.actors
            .values()
            .map(|actor| (actor.runtime_id, actor.position))
            .collect()
    }

    /// Bed block under every sleeping actor: the streamed bed position, else the block it lies in.
    pub(crate) fn bed_sample_points(&self) -> Vec<(u64, [i32; 3])> {
        self.actors
            .values()
            .filter(|actor| actor.is_sleeping())
            .map(|actor| {
                let block = match actor.metadata.get(&KEY_BED_POSITION) {
                    Some(ActorMetadataValue::BlockPosition(block)) => *block,
                    _ => actor.position.map(|axis| axis.floor() as i32),
                };
                (actor.runtime_id, block)
            })
            .collect()
    }

    /// Replaces every actor's sampled bed rotation with `(runtime_id, degrees)` samples.
    pub(crate) fn set_bed_rotations(&mut self, samples: &[(u64, f32)]) {
        for actor in self.actors.values_mut() {
            actor.status.sleep_rotation = None;
        }
        for &(runtime_id, degrees) in samples {
            if let Some(actor) = self.actors.get_mut(&runtime_id) {
                actor.status.sleep_rotation = Some(degrees);
            }
        }
    }

    /// Stores `(runtime_id, in_water, in_lava)` samples on their actors.
    pub(crate) fn set_fluids(&mut self, samples: &[(u64, bool, bool)]) {
        for &(runtime_id, water, lava) in samples {
            if let Some(actor) = self.actors.get_mut(&runtime_id) {
                actor.status.fluid = Some((water, lava));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{RideSeat, SeatDefaults, seat_world_offset};

    #[test]
    fn default_seats_follow_rider_count_and_index() {
        let mut defaults = SeatDefaults::default();
        let seat = |z, min, max| RideSeat {
            position: [0.0, 1.0, z],
            min_riders: min,
            max_riders: max,
        };
        defaults.insert("minecraft:camel", vec![seat(0.5, 0, 2), seat(-0.5, 1, 2)]);
        assert_eq!(
            defaults.seat("minecraft:camel", 1, 0),
            Some([0.0, 1.0, 0.5])
        );
        assert_eq!(
            defaults.seat("minecraft:camel", 2, 1),
            Some([0.0, 1.0, -0.5])
        );
        assert_eq!(defaults.seat("minecraft:pig", 1, 0), None);
    }

    #[test]
    fn seat_offset_rotates_about_the_vertical_axis() {
        let world = seat_world_offset([1.0, 0.5, 0.0], 90.0);
        assert!((world[0]).abs() < 1.0e-6 && (world[2] - 1.0).abs() < 1.0e-6);
        assert_eq!(world[1], 0.5);
        assert_eq!(seat_world_offset([0.0, 0.0, 2.0], 0.0), [0.0, 0.0, 2.0]);
    }
}
