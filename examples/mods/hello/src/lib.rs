//! Template: retained JSON-UI label and a host-assigned local keybind.

use mod_api::bindings::{
    Guest,
    cinnabar::extension::{hud, input},
};

struct Hello;

impl Guest for Hello {
    /// Publishes the initial label without touching the client's game state.
    fn init() {
        let _ = hud::set_label("Hello from a Cinnabar mod. Press the demo key.");
    }

    /// Reacts to the local action without synthesizing gameplay input.
    fn frame() {
        if input::demo_pressed() {
            let _ = hud::set_label("Hello! The mod received your keybind.");
        }
    }
}

mod_api::bindings::export!(Hello with_types_in mod_api::bindings);
