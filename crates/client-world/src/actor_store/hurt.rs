use protocol::{ActorMetadataValue, ActorStatusEvent, ActorStatusKind, ActorTakeItemEvent};

use super::{ActorApplyResult, ActorSnapshot, ActorStore};

/// Ticks the hurt tint and hurt-driven animations stay active; needs independent measurement.
pub const HURT_DURATION_TICKS: u8 = 10;
/// Ticks a dying actor takes to tip fully over; needs independent measurement.
pub const DEATH_DURATION_TICKS: u8 = 20;

/// Ticks a picked-up item takes to reach its collector; needs independent measurement.
pub const PICKUP_DURATION_TICKS: u8 = 3;

const HURT_DIRECTION_METADATA_KEY: u32 = 12;

/// A dropped item flying to the actor that collected it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ActorPickup {
    pub collector_runtime_id: u64,
    /// Ticks elapsed, saturating at [`PICKUP_DURATION_TICKS`].
    pub ticks: u8,
}

/// Client-derived damage and death presentation state, advanced per tick.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct ActorStatus {
    /// Ticks of hurt tint remaining.
    pub hurt_time: u8,
    /// Server-streamed hurt direction, when the server provides one.
    pub hurt_direction: Option<f32>,
    /// Ticks elapsed since death, saturating at [`DEATH_DURATION_TICKS`].
    pub death_time: u8,
    pub dead: bool,
    /// Ticks since the actor spawned; drives dropped-item spin and bob phase.
    pub age_ticks: u32,
    pub pickup: Option<ActorPickup>,
}

impl ActorStatus {
    /// Whether the red damage overlay should tint the actor this frame.
    #[must_use]
    pub fn overlay_active(&self) -> bool {
        self.hurt_time > 0 || self.dead
    }

    /// Death tip-over progress in `0..=1` at `partial_tick`, or `None` while alive.
    #[must_use]
    pub fn death_progress(&self, partial_tick: f32) -> Option<f32> {
        if !self.dead {
            return None;
        }
        let ticks = f32::from(self.death_time) + partial_tick.clamp(0.0, 1.0);
        Some((ticks / f32::from(DEATH_DURATION_TICKS)).clamp(0.0, 1.0))
    }

    pub(super) fn tick(&mut self) {
        self.age_ticks = self.age_ticks.saturating_add(1);
        if let Some(pickup) = &mut self.pickup {
            pickup.ticks = pickup.ticks.saturating_add(1).min(PICKUP_DURATION_TICKS);
        }
        self.hurt_time = self.hurt_time.saturating_sub(1);
        if self.dead && self.death_time < DEATH_DURATION_TICKS {
            self.death_time += 1;
        }
    }

    fn die(&mut self) {
        self.dead = true;
        self.hurt_time = HURT_DURATION_TICKS;
    }

    fn revive(&mut self) {
        self.hurt_time = 0;
        self.hurt_direction = None;
        self.death_time = 0;
        self.dead = false;
    }
}

impl ActorSnapshot {
    /// Marks the actor dead when its health attribute reaches zero and alive when it recovers.
    pub(super) fn sync_status_from_health(&mut self) {
        let Some(health) = self.attributes.get("minecraft:health") else {
            return;
        };
        if !health.current.is_finite() {
            return;
        }
        if health.current <= 0.0 {
            if !self.status.dead {
                self.status.die();
            }
        } else if self.status.dead {
            self.status.revive();
        }
    }

    fn streamed_hurt_direction(&self) -> Option<f32> {
        match self.metadata.get(&HURT_DIRECTION_METADATA_KEY)? {
            ActorMetadataValue::Byte(value) => Some(f32::from(*value)),
            ActorMetadataValue::Short(value) => Some(f32::from(*value)),
            ActorMetadataValue::Int(value) => Some(*value as f32),
            _ => None,
        }
    }
}

impl ActorStore {
    pub(super) fn apply_status(&mut self, event: ActorStatusEvent) -> ActorApplyResult {
        let Some(actor) = self.actors.get_mut(&event.runtime_id) else {
            return ActorApplyResult::MissingActor;
        };
        match event.kind {
            ActorStatusKind::Hurt => {
                actor.status.hurt_time = HURT_DURATION_TICKS;
                actor.status.hurt_direction = actor.streamed_hurt_direction();
            }
            ActorStatusKind::Death => {
                if !actor.status.dead {
                    actor.status.die();
                }
            }
            ActorStatusKind::SpawnAlive => actor.status.revive(),
            // Particle-only kinds have no retained actor state.
            _ => {}
        }
        ActorApplyResult::Updated
    }

    pub(super) fn apply_take_item(&mut self, event: ActorTakeItemEvent) -> ActorApplyResult {
        let Some(item) = self.actors.get_mut(&event.item_runtime_id) else {
            return ActorApplyResult::MissingActor;
        };
        item.status.pickup.get_or_insert(ActorPickup {
            collector_runtime_id: event.collector_runtime_id,
            ticks: 0,
        });
        ActorApplyResult::Updated
    }

    /// Hurt direction retained by the most recent Hurt event for `runtime_id`.
    pub(crate) fn hurt_direction(&self, runtime_id: u64) -> Option<f32> {
        self.actors.get(&runtime_id)?.status.hurt_direction
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hurt_counts_down_and_death_saturates() {
        let mut status = ActorStatus::default();
        status.hurt_time = HURT_DURATION_TICKS;
        for _ in 0..HURT_DURATION_TICKS {
            assert!(status.overlay_active());
            status.tick();
        }
        assert!(!status.overlay_active());

        status.die();
        for _ in 0..(DEATH_DURATION_TICKS + 5) {
            status.tick();
        }
        assert_eq!(status.death_time, DEATH_DURATION_TICKS);
        assert_eq!(status.death_progress(0.9), Some(1.0));
        assert!(status.overlay_active());
    }

    fn spawn() -> protocol::ActorEvent {
        protocol::ActorEvent::Spawn(protocol::ActorSpawnEvent {
            dimension: 0,
            unique_id: 5,
            runtime_id: 7,
            kind: protocol::ActorKind::Entity {
                identifier: "minecraft:cow".into(),
            },
            position: [0.0; 3],
            velocity: [0.0; 3],
            pitch: 0.0,
            yaw: 0.0,
            head_yaw: 0.0,
            body_yaw: 0.0,
            held_item: Default::default(),
            metadata: std::sync::Arc::from([]),
            attributes: std::sync::Arc::from([]),
            properties: std::sync::Arc::from([]),
            links: std::sync::Arc::from([]),
        })
    }

    fn status(kind: ActorStatusKind) -> protocol::ActorEvent {
        protocol::ActorEvent::Status(ActorStatusEvent {
            runtime_id: 7,
            kind,
            data: 0,
        })
    }

    #[test]
    fn hurt_event_arms_the_countdown_and_ticks_down() {
        let mut store = ActorStore::new(1, 0);
        store.apply(1, 1, spawn());
        assert_eq!(
            store.apply(1, 2, status(ActorStatusKind::Hurt)),
            ActorApplyResult::Updated
        );
        assert_eq!(store.get(7).unwrap().status.hurt_time, HURT_DURATION_TICKS);
        store.advance_interpolation_ticks(3);
        assert_eq!(
            store.get(7).unwrap().status.hurt_time,
            HURT_DURATION_TICKS - 3
        );
    }

    #[test]
    fn take_item_starts_pickup_and_saturates() {
        let mut store = ActorStore::new(1, 0);
        store.apply(1, 1, spawn());
        let take = protocol::ActorEvent::TakeItem(ActorTakeItemEvent {
            item_runtime_id: 7,
            collector_runtime_id: 99,
        });
        assert_eq!(store.apply(1, 2, take), ActorApplyResult::Updated);
        store.advance_interpolation_ticks(u32::from(PICKUP_DURATION_TICKS) + 4);
        let status = store.get(7).unwrap().status;
        assert_eq!(status.pickup.map(|pickup| pickup.ticks), Some(PICKUP_DURATION_TICKS));
        assert_eq!(status.pickup.map(|pickup| pickup.collector_runtime_id), Some(99));
    }

    #[test]
    fn death_event_for_unknown_actor_is_missing() {
        let mut store = ActorStore::new(1, 0);
        assert_eq!(
            store.apply(1, 1, status(ActorStatusKind::Death)),
            ActorApplyResult::MissingActor
        );
    }

    #[test]
    fn revive_clears_death() {
        let mut status = ActorStatus::default();
        status.die();
        status.revive();
        assert_eq!(status.death_progress(0.0), None);
        assert!(!status.overlay_active());
    }
}
