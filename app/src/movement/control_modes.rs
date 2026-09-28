//! Latched sprint and sneak state: sprint key, double-tap forward, toggle options and
//! the conditions that end a sprint.

use std::time::Duration;

/// Provisional double-tap window; needs independent measurement.
const DOUBLE_TAP_WINDOW: Duration = Duration::from_millis(350);
/// Food level at or below which survival sprinting is refused.
pub(crate) const SPRINT_HUNGER_FLOOR: u16 = 6;

/// Detects a second press inside the double-tap window.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct DoubleTap {
    last_press: Option<Duration>,
}

impl DoubleTap {
    /// Records a press at `now`; true when it completes a double-tap.
    pub(crate) fn press(&mut self, now: Duration) -> bool {
        let double = self
            .last_press
            .is_some_and(|last| now.saturating_sub(last) <= DOUBLE_TAP_WINDOW);
        // A completed double-tap must not chain into a triple.
        self.last_press = if double { None } else { Some(now) };
        double
    }

    pub(crate) fn reset(&mut self) {
        self.last_press = None;
    }
}

/// One render frame of sprint/sneak-relevant facts.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct ControlObservation {
    pub now: Duration,
    /// Forward axis after device normalization; positive is forward.
    pub forward: f32,
    pub sprint_pressed: bool,
    pub sprint_held: bool,
    pub sneak_pressed: bool,
    pub sneak_held: bool,
    pub toggle_sprint: bool,
    pub toggle_sneak: bool,
    /// Something external forbids sprinting (hunger, item use, blindness).
    pub sprint_blocked: bool,
    /// The previous completed tick collided horizontally.
    pub horizontal_collision: bool,
    /// Ability flight is active, where sneak means descend and never latches.
    pub flying: bool,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct ControlOutput {
    pub sprint_request: bool,
    pub sneaking: bool,
}

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct ControlModes {
    sprinting: bool,
    sprint_toggled: bool,
    sneak_toggled: bool,
    was_moving_forward: bool,
    last_forward_press: Option<Duration>,
}

impl ControlModes {
    pub(crate) fn reset(&mut self) {
        *self = Self::default();
    }

    pub(crate) fn update(&mut self, observed: ControlObservation) -> ControlOutput {
        let moving_forward = observed.forward > 0.0;
        let mut double_tap = false;
        if moving_forward && !self.was_moving_forward {
            double_tap = self
                .last_forward_press
                .is_some_and(|last| observed.now.saturating_sub(last) <= DOUBLE_TAP_WINDOW);
            self.last_forward_press = Some(observed.now);
        }
        self.was_moving_forward = moving_forward;

        let sneaking = if observed.toggle_sneak && !observed.flying {
            if observed.sneak_pressed {
                self.sneak_toggled = !self.sneak_toggled;
            }
            self.sneak_toggled
        } else {
            self.sneak_toggled = false;
            observed.sneak_held
        };

        if observed.toggle_sprint {
            if observed.sprint_pressed {
                self.sprint_toggled = !self.sprint_toggled;
                if !self.sprint_toggled {
                    self.sprinting = false;
                }
            }
        } else {
            self.sprint_toggled = false;
        }

        let can_sprint = moving_forward
            && !sneaking
            && !observed.sprint_blocked
            && !observed.horizontal_collision;
        if !can_sprint {
            self.sprinting = false;
        } else if (observed.sprint_held && !observed.toggle_sprint)
            || double_tap
            || self.sprint_toggled
        {
            self.sprinting = true;
        }
        ControlOutput {
            sprint_request: self.sprinting,
            sneaking,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(millis: u64, forward: f32) -> ControlObservation {
        ControlObservation {
            now: Duration::from_millis(millis),
            forward,
            ..ControlObservation::default()
        }
    }

    #[test]
    fn double_tap_detector_does_not_chain_into_a_triple() {
        let mut tap = DoubleTap::default();
        assert!(!tap.press(Duration::from_millis(0)));
        assert!(tap.press(Duration::from_millis(200)));
        assert!(!tap.press(Duration::from_millis(300)));
        assert!(!tap.press(Duration::from_millis(1000)));
    }

    #[test]
    fn sprint_key_latches_until_forward_input_ends() {
        let mut modes = ControlModes::default();
        let held = ControlObservation {
            sprint_held: true,
            ..frame(0, 1.0)
        };
        assert!(modes.update(held).sprint_request);
        assert!(modes.update(frame(50, 1.0)).sprint_request);
        assert!(!modes.update(frame(100, 0.0)).sprint_request);
        assert!(!modes.update(frame(900, 1.0)).sprint_request);
    }

    #[test]
    fn double_tap_forward_sprints_only_inside_the_window() {
        let mut modes = ControlModes::default();
        modes.update(frame(0, 1.0));
        modes.update(frame(100, 0.0));
        assert!(modes.update(frame(200, 1.0)).sprint_request);

        let mut slow = ControlModes::default();
        slow.update(frame(0, 1.0));
        slow.update(frame(100, 0.0));
        assert!(!slow.update(frame(900, 1.0)).sprint_request);
    }

    #[test]
    fn collision_sneak_and_block_end_sprint_without_flicker() {
        let mut modes = ControlModes::default();
        let sprint = ControlObservation {
            sprint_held: true,
            ..frame(0, 1.0)
        };
        assert!(modes.update(sprint).sprint_request);
        let bump = ControlObservation {
            horizontal_collision: true,
            ..sprint
        };
        assert!(!modes.update(bump).sprint_request);
        assert!(!modes.update(bump).sprint_request);
        assert!(modes.update(sprint).sprint_request);

        let sneak = ControlObservation {
            sneak_held: true,
            ..sprint
        };
        let output = modes.update(sneak);
        assert!(output.sneaking && !output.sprint_request);

        let hungry = ControlObservation {
            sprint_blocked: true,
            ..sprint
        };
        assert!(!modes.update(hungry).sprint_request);
    }

    #[test]
    fn toggle_sprint_survives_key_release_and_toggles_off_on_second_press() {
        let mut modes = ControlModes::default();
        let press = ControlObservation {
            toggle_sprint: true,
            sprint_pressed: true,
            sprint_held: true,
            ..frame(0, 1.0)
        };
        assert!(modes.update(press).sprint_request);
        let released = ControlObservation {
            toggle_sprint: true,
            ..frame(50, 1.0)
        };
        assert!(modes.update(released).sprint_request);
        assert!(
            !modes
                .update(ControlObservation {
                    now: Duration::from_millis(100),
                    ..press
                })
                .sprint_request
        );
    }

    #[test]
    fn toggle_sneak_latches_and_flight_never_latches() {
        let mut modes = ControlModes::default();
        let press = ControlObservation {
            toggle_sneak: true,
            sneak_pressed: true,
            sneak_held: true,
            ..ControlObservation::default()
        };
        assert!(modes.update(press).sneaking);
        let released = ControlObservation {
            toggle_sneak: true,
            ..ControlObservation::default()
        };
        assert!(modes.update(released).sneaking);
        let flying = ControlObservation {
            flying: true,
            ..released
        };
        assert!(!modes.update(flying).sneaking);
        assert!(!modes.update(released).sneaking);
    }

    #[test]
    fn toggle_disabled_follows_the_held_button() {
        let mut modes = ControlModes::default();
        let held = ControlObservation {
            sneak_held: true,
            ..ControlObservation::default()
        };
        assert!(modes.update(held).sneaking);
        assert!(!modes.update(ControlObservation::default()).sneaking);
    }
}
