//! Desktop key remapping uses the same physical controls as the gameplay router.

use super::SettingsOptions;
use semantic_input::{Action, ControlSettings, InputContext, PhysicalControl};

pub(crate) const KEY_BINDINGS: &[(Action, &str)] = &[
    (Action::Attack, "key.attack"),
    (Action::Use, "key.use"),
    (Action::MoveForward, "key.forward"),
    (Action::MoveBackward, "key.back"),
    (Action::MoveLeft, "key.left"),
    (Action::MoveRight, "key.right"),
    (Action::Jump, "key.jump"),
    (Action::Sneak, "key.sneak"),
    (Action::Sprint, "key.sprint"),
    (Action::CyclePerspective, "key.togglePerspective"),
    (Action::Hotbar1, "key.hotbar.1"),
    (Action::Hotbar2, "key.hotbar.2"),
    (Action::Hotbar3, "key.hotbar.3"),
    (Action::Hotbar4, "key.hotbar.4"),
    (Action::Hotbar5, "key.hotbar.5"),
    (Action::Hotbar6, "key.hotbar.6"),
    (Action::Hotbar7, "key.hotbar.7"),
    (Action::Hotbar8, "key.hotbar.8"),
    (Action::Hotbar9, "key.hotbar.9"),
];

impl SettingsOptions {
    /// Returns the selected keyboard or mouse control, using the router defaults initially.
    pub(crate) fn key_control(&self, index: usize) -> Option<PhysicalControl> {
        let (action, name) = KEY_BINDINGS.get(index)?;
        self.keys
            .get(*name)
            .and_then(|code| decode_control(*code))
            .or_else(|| {
                ControlSettings::default()
                    .bindings()
                    .iter()
                    .find(|binding| {
                        binding.context == InputContext::Gameplay
                            && binding.action == *action
                            && matches!(
                                binding.chord.control,
                                PhysicalControl::KeyboardUsage(_) | PhysicalControl::MouseButton(_)
                            )
                    })
                    .map(|binding| binding.chord.control)
            })
    }

    /// Saves a mapping only if the complete gameplay binding set remains valid.
    pub(crate) fn remap(&mut self, index: usize, control: PhysicalControl) -> bool {
        let Some((_, name)) = KEY_BINDINGS.get(index) else {
            return false;
        };
        let Some(code) = encode_control(control) else {
            return false;
        };
        let previous = self.keys.insert((*name).to_owned(), code);
        if self.controls().is_err() {
            match previous {
                Some(code) => {
                    self.keys.insert((*name).to_owned(), code);
                }
                None => {
                    self.keys.remove(*name);
                }
            }
            return false;
        }
        true
    }

    /// Restores one action's default control while preserving other remaps.
    pub(crate) fn reset_key(&mut self, index: usize) -> bool {
        let Some((_, name)) = KEY_BINDINGS.get(index) else {
            return false;
        };
        let previous = self.keys.remove(*name);
        if self.controls().is_err()
            && let Some(previous) = previous
        {
            self.keys.insert((*name).to_owned(), previous);
            return false;
        }
        true
    }

    /// Rebuilds and validates the gameplay bindings before handing them to the router.
    pub(super) fn controls(&self) -> Result<ControlSettings, semantic_input::BindingError> {
        let original = ControlSettings::default();
        let mut bindings = original.bindings().to_vec();
        for (action, name) in KEY_BINDINGS {
            let Some(control) = self.keys.get(*name).and_then(|code| decode_control(*code)) else {
                continue;
            };
            let mut replaced = false;
            bindings.retain_mut(|binding| {
                if binding.action != *action
                    || binding.context != InputContext::Gameplay
                    || !matches!(
                        binding.chord.control,
                        PhysicalControl::KeyboardUsage(_) | PhysicalControl::MouseButton(_)
                    )
                {
                    return true;
                }
                if replaced {
                    return false;
                }
                binding.chord.control = control;
                replaced = true;
                true
            });
        }
        ControlSettings::new(
            bindings,
            original.mouse_sensitivity,
            original.gamepad_look_sensitivity,
            original.touch_look_sensitivity,
            original.invert_mouse_y,
            original.invert_gamepad_y,
            original.gamepad_move_deadzone,
            original.gamepad_look_deadzone,
        )
    }
}

/// Encodes keyboard and mouse controls without serializing engine enum representations.
fn encode_control(control: PhysicalControl) -> Option<u16> {
    match control {
        PhysicalControl::KeyboardUsage(code) if (0x04..=0xe7).contains(&code) => Some(code),
        PhysicalControl::MouseButton(button) if (1..=8).contains(&button) => {
            Some(0x100 + u16::from(button))
        }
        _ => None,
    }
}

/// Rejects persisted values outside the supported keyboard and mouse ranges.
fn decode_control(code: u16) -> Option<PhysicalControl> {
    match code {
        0x04..=0xe7 => Some(PhysicalControl::KeyboardUsage(code)),
        0x101..=0x108 => Some(PhysicalControl::MouseButton((code - 0x100) as u8)),
        _ => None,
    }
}

/// Displays USB controls in the keyboard layout's common desktop notation.
pub(crate) fn key_name(control: PhysicalControl) -> String {
    match control {
        PhysicalControl::KeyboardUsage(code @ 0x04..=0x1d) => {
            char::from(b'A' + (code - 4) as u8).to_string()
        }
        PhysicalControl::KeyboardUsage(code @ 0x1e..=0x26) => (code - 0x1d).to_string(),
        PhysicalControl::KeyboardUsage(0x27) => "0".to_owned(),
        PhysicalControl::KeyboardUsage(0x2c) => "Space".to_owned(),
        PhysicalControl::KeyboardUsage(0xe0) => "Left Control".to_owned(),
        PhysicalControl::KeyboardUsage(0xe1) => "Left Shift".to_owned(),
        PhysicalControl::KeyboardUsage(code @ 0x3a..=0x45) => format!("F{}", code - 0x39),
        PhysicalControl::MouseButton(button) => format!("Mouse {button}"),
        PhysicalControl::KeyboardUsage(code) => format!("Key {code:02X}"),
        _ => String::new(),
    }
}
