use bevy::prelude::Resource;

/// Largest effective speed that stays inside the collision query extent.
const MAX_SIMULABLE_MOVEMENT_SPEED: f64 = sim::MAX_COLLISION_QUERY_EXTENT / 4.0;

/// Attribute current and the native sprint modifier currently installed on it.
#[derive(Debug, Default, Clone, Copy, PartialEq)]
pub(crate) struct EffectiveMovementSpeed {
    current: Option<f64>,
    sprinting: bool,
    sprint_modifier: Option<f32>,
}

impl EffectiveMovementSpeed {
    pub(crate) fn authoritative(
        current: f64,
        sprint_modifier: Option<f32>,
        sprinting: bool,
    ) -> Self {
        Self {
            current: Some(current),
            sprinting,
            sprint_modifier,
        }
    }

    /// Metadata changes the actor flag without installing an attribute modifier.
    fn adopt_server_sprinting(&mut self, sprinting: Option<bool>) {
        if let Some(sprinting) = sprinting {
            self.sprinting = sprinting;
        }
    }

    /// Native LocalPlayer::setSprinting is edge-triggered; Mob adds/removes only
    /// its identified modifier. An attribute packet replaces that modifier set.
    pub(crate) fn set_sprinting(&mut self, sprinting: bool) {
        if self.sprinting == sprinting {
            return;
        }
        self.sprinting = sprinting;
        if sprinting {
            if self.sprint_modifier.is_none() {
                let factor = sim::SPRINT_SPEED_MULTIPLIER as f32;
                self.current = self
                    .current
                    .map(|current| f64::from(current as f32 * factor));
                self.sprint_modifier = Some(factor);
            }
        } else if let Some(factor) = self.sprint_modifier.take() {
            self.current = self
                .current
                .map(|current| f64::from(current as f32 / factor));
        }
    }

    /// The simulator's public input uses pre-sprint speed. Cancel its one fixed
    /// multiplier so its result reads our effective attribute current exactly once.
    pub(crate) fn prediction_speed(self) -> Option<f64> {
        self.current
            .map(|current| prediction_speed(current, self.sprinting))
    }
}

fn prediction_speed(current: f64, sprinting: bool) -> f64 {
    if sprinting {
        f64::from(current as f32 / sim::SPRINT_SPEED_MULTIPLIER as f32)
    } else {
        current
    }
}

/// An authoritative flag or mode rewrite preserves effective current; it does
/// not create a local sprint modifier edge.
pub(crate) fn preserve_effective_speed(input: &mut sim::MovementInput, previous_sprinting: bool) {
    if previous_sprinting == input.sprinting {
        return;
    }
    if let Some(speed) = input.movement_speed {
        let current = if previous_sprinting {
            f64::from(speed as f32 * sim::SPRINT_SPEED_MULTIPLIER as f32)
        } else {
            speed
        };
        input.movement_speed = Some(prediction_speed(current, input.sprinting));
    }
}

#[derive(Debug, Default, Resource)]
pub(crate) struct LocalMovementSpeedAuthority {
    session_id: u64,
    dimension: i32,
    last_sequence: Option<u64>,
    speed: EffectiveMovementSpeed,
}

impl LocalMovementSpeedAuthority {
    pub(crate) fn begin_session(&mut self, session_id: u64, dimension: i32) {
        self.session_id = session_id;
        self.dimension = dimension;
        self.last_sequence = None;
        self.speed = EffectiveMovementSpeed::default();
    }

    pub(crate) fn replace_dimension(&mut self, session_id: u64, dimension: i32) {
        if session_id != self.session_id {
            return;
        }
        self.dimension = dimension;
        self.last_sequence = None;
        self.speed = EffectiveMovementSpeed::default();
    }

    pub(crate) fn apply(
        &mut self,
        session_id: u64,
        sequence: u64,
        dimension: i32,
        current: f64,
        sprint_modifier: Option<f32>,
    ) -> bool {
        if session_id != self.session_id
            || dimension != self.dimension
            || self.last_sequence.is_some_and(|last| sequence <= last)
        {
            return false;
        }
        self.last_sequence = Some(sequence);
        if !(0.0..=MAX_SIMULABLE_MOVEMENT_SPEED).contains(&current)
            || sprint_modifier.is_some_and(|factor| {
                !factor.is_finite()
                    || factor <= 0.0
                    || !(0.0..=MAX_SIMULABLE_MOVEMENT_SPEED)
                        .contains(&f64::from(current as f32 / factor))
            })
        {
            super::diagnostics::note_skipped_authority("movement_speed", current);
            return false;
        }
        self.speed =
            EffectiveMovementSpeed::authoritative(current, sprint_modifier, self.speed.sprinting);
        true
    }

    pub(crate) const fn current(&self) -> Option<f64> {
        self.speed.current
    }

    pub(crate) fn prediction_speed(&self) -> Option<f64> {
        self.speed.prediction_speed()
    }

    pub(crate) fn set_sprinting(&mut self, sprinting: bool) {
        self.speed.set_sprinting(sprinting);
    }

    pub(crate) fn adopt_server_sprinting(&mut self, sprinting: Option<bool>) {
        self.speed.adopt_server_sprinting(sprinting);
    }

    pub(crate) fn adopt_replayed_speed(&mut self, speed: EffectiveMovementSpeed) {
        self.speed = speed;
    }
}

#[cfg(test)]
#[path = "speed_authority_tests.rs"]
mod tests;
