use super::{java_swing_duration, swing_duration};
use crate::movement::{LocalMovementEffectTimeline, MAX_LOCAL_PHYSICS_TICKS_PER_FRAME};
use client_world::LocalSwingProgress;

/// Packet admission and both animation counters on the committed local simulation clock.
#[derive(Debug, Default, Clone)]
pub struct SwingTracker {
    authority: Option<(u64, u64)>,
    completed_tick: Option<u64>,
    attempted_tick: Option<u64>,
    started: Option<i32>,
    states: [Counter; 2],
    history: [(i32, i32); MAX_LOCAL_PHYSICS_TICKS_PER_FRAME],
    guard_history: [i32; MAX_LOCAL_PHYSICS_TICKS_PER_FRAME],
    history_end: Option<u64>,
    history_len: usize,
}

#[derive(Debug, Clone)]
struct Counter {
    counter: Option<i32>,
    progress: [f32; 2],
    duration: i32,
}

impl Default for Counter {
    /// Idle counters use the shared native base duration.
    fn default() -> Self {
        Self {
            counter: None,
            progress: [0.0; 2],
            duration: client_world::ACTOR_SWING_TICKS,
        }
    }
}

impl Counter {
    /// Admission reads the counter before this tick's increment.
    fn try_start(&mut self, duration: i32) -> bool {
        let accepted = self
            .counter
            .is_none_or(|counter| counter < 0 || counter >= duration.max(1) / 2);
        if accepted {
            self.counter = Some(-1);
        }
        accepted
    }

    /// Skips an interval with one denominator while retaining the final two native samples.
    fn advance(&mut self, count: u64, duration: i32) {
        if count == 0 {
            return;
        }
        self.duration = duration.max(1);
        let counter = self.counter;
        let value = |steps: u64| {
            counter.and_then(|counter| {
                let next = i128::from(counter) + i128::from(steps);
                (next < i128::from(self.duration)).then_some(next as i32)
            })
        };
        let previous = if count == 1 {
            self.progress[1]
        } else {
            value(count - 1).map_or(0.0, |counter| counter.max(0) as f32 / self.duration as f32)
        };
        self.counter = value(count);
        self.progress = [
            previous,
            self.counter
                .map_or(0.0, |counter| counter.max(0) as f32 / self.duration as f32),
        ];
    }
}

impl SwingTracker {
    /// Supplies each committed tick's post-expiry denominators and resets on a new movement authority.
    pub fn sync_ticks(
        &mut self,
        authority: (u64, u64),
        completed_tick: u64,
        effects: &LocalMovementEffectTimeline,
    ) {
        if self.authority != Some(authority) {
            *self = Self {
                authority: Some(authority),
                ..Self::default()
            };
        }
        self.history_end = Some(completed_tick);
        self.history_len = effects.recent_tick_count();
        for distance in 0..self.history_len {
            let (before, after) = effects.mining_tick(
                completed_tick.saturating_sub(distance as u64),
                completed_tick,
            );
            self.guard_history[distance] = java_swing_duration(before);
            self.history[distance] = (swing_duration(after), java_swing_duration(after));
        }
    }

    /// Attempts both native animations independently; the result admits the Bedrock wire packet.
    pub fn try_swing(&mut self, tick: u64, duration: i32) -> bool {
        if self.authority.is_none() && self.attempted_tick.is_some_and(|previous| tick < previous) {
            *self = Self::default();
        }
        if self.attempted_tick == Some(tick)
            || self
                .completed_tick
                .is_some_and(|completed| tick <= completed)
        {
            return false;
        }
        if tick > 0 {
            self.advance_to(tick - 1);
        }
        let java_duration = self.guard_java_duration(tick).unwrap_or(duration);
        let bedrock = self.states[0].try_start(duration);
        self.states[1].try_start(java_duration);
        self.states[0].duration = duration.max(1);
        self.states[1].duration = java_duration.max(1);
        self.attempted_tick = Some(tick);
        if bedrock {
            self.started = Some(duration);
        }
        bedrock
    }

    /// Returns the latest accepted wire duration for callers that use the scalar start API.
    pub fn take_started(&mut self) -> Option<i32> {
        self.started.take()
    }

    /// Finishes every committed tick before publishing each mode's previous/current samples.
    pub fn published_progress(&mut self, completed_tick: u64) -> LocalSwingProgress {
        self.advance_to(completed_tick);
        self.started = None;
        LocalSwingProgress {
            bedrock: self.states[0].progress,
            java: self.states[1].progress,
            frame_alpha: None,
        }
    }

    /// The pre-expiry Java duration is retained alongside the post-expiry animation history.
    fn guard_java_duration(&self, tick: u64) -> Option<i32> {
        let distance = self.history_end?.checked_sub(tick)? as usize;
        (distance < self.history_len).then(|| self.guard_history[distance])
    }

    /// Advances only unconsumed local ticks, with bounded work even after a large tick jump.
    fn advance_to(&mut self, target: u64) {
        let start = self
            .completed_tick
            .and_then(|tick| tick.checked_add(1))
            .or(self.attempted_tick);
        let Some(mut next) = start else {
            self.completed_tick = Some(target);
            return;
        };
        if next > target {
            return;
        }
        let first = self
            .history_end
            .filter(|_| self.history_len > 0)
            .map(|end| end.saturating_sub(self.history_len as u64 - 1));
        if let Some(first) = first {
            if next < first {
                let last = target.min(first - 1);
                for state in &mut self.states {
                    state.advance(last - next + 1, state.duration);
                }
                next = last.saturating_add(1);
            }
            if next <= target {
                for tick in next..=target.min(self.history_end.unwrap()) {
                    let distance = (self.history_end.unwrap() - tick) as usize;
                    let durations = self.history[distance];
                    self.states[0].advance(1, durations.0);
                    self.states[1].advance(1, durations.1);
                    next = tick.saturating_add(1);
                }
            }
        }
        if next <= target {
            for state in &mut self.states {
                state.advance(target - next + 1, state.duration);
            }
        }
        self.completed_tick = Some(target);
    }
}
