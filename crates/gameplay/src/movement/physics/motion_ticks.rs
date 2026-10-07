//! Presentation observations stay independent of the server correction window.

use super::*;

/// Completed movement facts used by local motion presentation.
#[derive(Debug, Clone, Copy)]
pub struct PhysicsMotionSample {
    pub tick: u64,
    pub position: [f32; 3],
    pub movement: [f32; 3],
    pub velocity: [f32; 3],
    pub grounded_after_tick: bool,
    pub sneaking: bool,
}

#[derive(Debug, Clone, Copy)]
pub(super) struct CompletedMotionTick {
    sample: PhysicsMotionSample,
    environment: sim::MovementEnvironment,
    entry_velocity: [f32; 3],
}

impl CompletedMotionTick {
    /// Copies only presentation facts, without collision provenance or replay inputs.
    fn new(sample: &PhysicsMovementSample, frame: &ControllerFrame) -> Self {
        Self {
            sample: PhysicsMotionSample {
                tick: sample.tick,
                position: sample.position,
                movement: sample.movement,
                velocity: sample.velocity,
                grounded_after_tick: sample.grounded_after_tick,
                sneaking: sample.processed.sneaking,
            },
            environment: frame.environment,
            entry_velocity: frame.entry_velocity,
        }
    }
}

/// Retains a full render frame's ticks without per-tick allocation.
pub(super) fn retain(
    ticks: &mut VecDeque<CompletedMotionTick>,
    sample: &PhysicsMovementSample,
    frame: &ControllerFrame,
) {
    if ticks.len() == MAX_LOCAL_PHYSICS_TICKS_PER_FRAME {
        ticks.pop_front();
    }
    ticks.push_back(CompletedMotionTick::new(sample, frame));
}

impl LocalPhysicsController {
    /// Visits completed ticks with pre-travel velocity and liquid contact.
    /// An absent cursor visits only the latest tick to prime a new observer.
    pub fn visit_completed_ticks(
        &self,
        after: Option<u64>,
        visit: &mut dyn FnMut(&PhysicsMotionSample, &sim::MovementEnvironment, [f32; 3]),
    ) {
        let after = after.or_else(|| {
            self.motion_ticks
                .back()
                .map(|tick| tick.sample.tick.saturating_sub(1))
        });
        for tick in &self.motion_ticks {
            if after.is_none_or(|after| tick.sample.tick > after) {
                visit(&tick.sample, &tick.environment, tick.entry_velocity);
            }
        }
    }

    /// Updates pending presentation observations after correction replay.
    pub(super) fn refresh_motion_ticks(&mut self) {
        for tick in &mut self.motion_ticks {
            if let (Some(sample), Some(frame)) = (
                self.sample_history
                    .iter()
                    .find(|sample| sample.tick == tick.sample.tick),
                self.controller_history
                    .iter()
                    .find(|frame| frame.tick == tick.sample.tick),
            ) {
                *tick = CompletedMotionTick::new(sample, frame);
            }
        }
    }
}
