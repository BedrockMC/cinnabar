//! Cardinal input belongs to an open wheel, even when it is the opening binding.
use bevy::{
    input::gamepad::{Gamepad, GamepadButton},
    prelude::Query,
};

pub(super) fn directional_navigation(pads: &Query<&Gamepad>) -> bool {
    pads.iter().any(|pad| {
        [
            GamepadButton::DPadUp,
            GamepadButton::DPadRight,
            GamepadButton::DPadDown,
            GamepadButton::DPadLeft,
        ]
        .into_iter()
        .any(|button| pad.just_pressed(button))
    })
}
