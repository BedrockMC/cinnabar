//! Device-specific bindings share the physical control codes used by gameplay.

use semantic_input::{Action, AxisDirection, PhysicalControl};

pub(crate) const GAMEPAD_OFFSET: usize = 0x8000;
pub(crate) const EXTRA_KEYS: &[(&str, PhysicalControl)] = &[
    ("key.inventory", PhysicalControl::KeyboardUsage(0x08)),
    ("key.chat", PhysicalControl::KeyboardUsage(0x17)),
    ("key.command", PhysicalControl::KeyboardUsage(0x38)),
    ("key.drop", PhysicalControl::KeyboardUsage(0x14)),
    ("key.pickItem", PhysicalControl::MouseButton(3)),
    ("key.screenshot", PhysicalControl::KeyboardUsage(0x3b)),
    ("key.fullscreen", PhysicalControl::KeyboardUsage(0x44)),
];
// R:v/VanillaClientInputMappingFactory.cpp:710,802,1094,1204.
pub(crate) const EXTRA_GAMEPAD: &[(&str, Option<PhysicalControl>)] = &[
    ("key.inventory", Some(PhysicalControl::GamepadButton(2))),
    ("key.chat", Some(PhysicalControl::GamepadButton(14))),
    ("key.drop", Some(PhysicalControl::GamepadButton(12))),
    ("key.pickItem", None),
];
pub(crate) const GAMEPAD_BINDINGS: &[(Action, &str)] = &[
    (Action::Attack, "key.attack"),
    (Action::Use, "key.use"),
    (Action::Jump, "key.jump"),
    (Action::Sneak, "key.sneak"),
    (Action::Sprint, "key.sprint"),
    (Action::CyclePerspective, "key.togglePerspective"),
    (Action::HotbarPrevious, "key.cycleItemLeft"),
    (Action::HotbarNext, "key.cycleItemRight"),
];

/// Keeps keyboard and gamepad bindings in disjoint persisted code ranges.
pub(super) fn encode_control(control: PhysicalControl) -> Option<u16> {
    match control {
        PhysicalControl::KeyboardUsage(code) if (0x04..=0xe7).contains(&code) => Some(code),
        PhysicalControl::MouseButton(button) if (1..=8).contains(&button) => {
            Some(0x100 + u16::from(button))
        }
        PhysicalControl::GamepadButton(button) if button <= 31 => Some(0x200 + u16::from(button)),
        PhysicalControl::GamepadAxis {
            axis: axis @ 4..=5,
            direction: AxisDirection::Positive,
        } => Some(0x300 + u16::from(axis)),
        _ => None,
    }
}

/// Rejects persisted controls the desktop capture path cannot produce.
pub(super) fn decode_control(code: u16) -> Option<PhysicalControl> {
    match code {
        0x04..=0xe7 => Some(PhysicalControl::KeyboardUsage(code)),
        0x101..=0x108 => Some(PhysicalControl::MouseButton((code - 0x100) as u8)),
        0x200..=0x21f => Some(PhysicalControl::GamepadButton((code - 0x200) as u8)),
        0x304..=0x305 => Some(PhysicalControl::GamepadAxis {
            axis: (code - 0x300) as u8,
            direction: AxisDirection::Positive,
        }),
        _ => None,
    }
}

/// Identifies gamepad controls without treating the movement sticks as remappable buttons.
pub(super) fn is_gamepad(control: PhysicalControl) -> bool {
    matches!(
        control,
        PhysicalControl::GamepadButton(_) | PhysicalControl::GamepadAxis { .. }
    )
}

/// Displays the pack's Xbox button glyph for a captured gamepad control.
pub(crate) fn gamepad_icon(control: PhysicalControl) -> &'static str {
    match control {
        PhysicalControl::GamepadButton(0) => "textures/ui/xbox_face_button_down",
        PhysicalControl::GamepadButton(1) => "textures/ui/xbox_face_button_right",
        PhysicalControl::GamepadButton(2) => "textures/ui/xbox_face_button_up",
        PhysicalControl::GamepadButton(3) => "textures/ui/xbox_face_button_left",
        PhysicalControl::GamepadButton(4) => "textures/ui/xbox_bumper_left",
        PhysicalControl::GamepadButton(5) => "textures/ui/xbox_bumper_right",
        PhysicalControl::GamepadButton(8) => "textures/ui/xbox_stick_left",
        PhysicalControl::GamepadButton(9) => "textures/ui/xbox_stick_right",
        PhysicalControl::GamepadButton(11) => "textures/ui/xbox_dpad_up",
        PhysicalControl::GamepadButton(12) => "textures/ui/xbox_dpad_down",
        PhysicalControl::GamepadButton(13) => "textures/ui/xbox_dpad_left",
        PhysicalControl::GamepadButton(14) => "textures/ui/xbox_dpad_right",
        PhysicalControl::GamepadAxis { axis: 4, .. } => "textures/ui/xbox_left_trigger",
        PhysicalControl::GamepadAxis { axis: 5, .. } => "textures/ui/xbox_right_trigger",
        _ => "",
    }
}

/// Resolves a gameplay UI control from the menu's persisted settings or startup defaults.
fn named_control(menu: Option<&crate::menu::MenuRuntime>, name: &str) -> Option<PhysicalControl> {
    menu.map_or_else(
        || super::SettingsOptions::default().named_key_control(name),
        |menu| menu.settings_options.named_key_control(name),
    )
}

/// Matches a keyboard event without changing the physical key used for text entry.
pub(crate) fn binding_key(
    menu: Option<&crate::menu::MenuRuntime>,
    name: &str,
    key: bevy::prelude::KeyCode,
) -> bool {
    crate::semantic_controls::keyboard_usage(key)
        .is_some_and(|code| named_control(menu, name) == Some(PhysicalControl::KeyboardUsage(code)))
}

/// Tests one configured keyboard or mouse action in the production device frame.
pub(crate) fn binding_pressed(
    menu: Option<&crate::menu::MenuRuntime>,
    name: &str,
    keys: &bevy::prelude::ButtonInput<bevy::prelude::KeyCode>,
    mouse: &bevy::prelude::ButtonInput<bevy::prelude::MouseButton>,
) -> bool {
    match named_control(menu, name) {
        Some(PhysicalControl::KeyboardUsage(code)) => keys
            .get_just_pressed()
            .any(|key| crate::semantic_controls::keyboard_usage(*key) == Some(code)),
        Some(PhysicalControl::MouseButton(code)) => mouse.get_just_pressed().any(|button| {
            crate::semantic_controls::physical::mouse_button_code(*button) == Some(code)
        }),
        _ => false,
    }
}

/// Matches mouse-only UI actions before keyboard events are processed.
pub(crate) fn binding_mouse(
    menu: Option<&crate::menu::MenuRuntime>,
    name: &str,
    mouse: &bevy::prelude::ButtonInput<bevy::prelude::MouseButton>,
) -> bool {
    let Some(PhysicalControl::MouseButton(code)) = named_control(menu, name) else {
        return false;
    };
    mouse
        .get_just_pressed()
        .any(|button| crate::semantic_controls::physical::mouse_button_code(*button) == Some(code))
}

/// Reads a gamepad UI action from the same persisted layout as the settings grid.
pub(crate) fn binding_gamepad(
    menu: Option<&crate::menu::MenuRuntime>,
    name: &str,
    pads: &bevy::prelude::Query<&bevy::input::gamepad::Gamepad>,
) -> bool {
    let Some(index) = EXTRA_GAMEPAD.iter().position(|(label, _)| *label == name) else {
        return false;
    };
    let control = menu.and_then(|menu| {
        menu.settings_options
            .key_control(GAMEPAD_OFFSET + GAMEPAD_BINDINGS.len() + index)
    });
    pads.iter().any(|pad| match control {
        Some(PhysicalControl::GamepadButton(code)) => {
            crate::semantic_controls::physical::TRANSLATED_GAMEPAD_BUTTONS
                .iter()
                .any(|(button_code, button)| *button_code == code && pad.just_pressed(*button))
        }
        Some(PhysicalControl::GamepadAxis { axis, .. }) => pad.just_pressed(if axis == 4 {
            bevy::input::gamepad::GamepadButton::LeftTrigger2
        } else {
            bevy::input::gamepad::GamepadButton::RightTrigger2
        }),
        _ => false,
    })
}

impl super::SettingsOptions {
    /// Applies the vanilla A/B and X/Y physical-button swaps after resolving a binding.
    pub(super) fn swap_gamepad_control(&self, control: PhysicalControl) -> PhysicalControl {
        let PhysicalControl::GamepadButton(button) = control else {
            return control;
        };
        let button = match button {
            0 | 1 if self.value("swap_gamepad_ab_buttons") != 0 => 1 - button,
            2 | 3 if self.value("swap_gamepad_xy_buttons") != 0 => 5 - button,
            _ => button,
        };
        PhysicalControl::GamepadButton(button)
    }

    /// Uses the same swap mapping for menu confirmation as for gameplay.
    pub(crate) fn gamepad_button(
        &self,
        button: bevy::input::gamepad::GamepadButton,
    ) -> bevy::input::gamepad::GamepadButton {
        let table = crate::semantic_controls::physical::TRANSLATED_GAMEPAD_BUTTONS;
        let Some((code, _)) = table.iter().find(|(_, candidate)| *candidate == button) else {
            return button;
        };
        let swapped = self.swap_gamepad_control(PhysicalControl::GamepadButton(*code));
        table
            .iter()
            .find(|(code, _)| swapped == PhysicalControl::GamepadButton(*code))
            .map_or(button, |(_, button)| *button)
    }
}
