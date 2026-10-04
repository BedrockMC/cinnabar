//! Host adapters for shared launcher settings.

mod actions;
mod chat;
mod control_bindings;
mod language;
mod reset;
mod runtime;
#[cfg(test)]
mod tests;

pub(crate) use control_bindings::{
    binding_gamepad, binding_key, binding_mouse, binding_pressed, gamepad_button,
};
pub(crate) use launcher::menu::settings_options::*;
