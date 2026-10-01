//! Settings actions stay separate from launcher navigation.

use crate::menu::{MenuAction, MenuRuntime};
use std::sync::Arc;

impl MenuRuntime {
    /// Applies settings navigation and edits without starting a game session.
    pub(in crate::menu) fn activate_settings(&mut self, action: MenuAction) {
        match action {
            MenuAction::SettingsResetChat => self.reset_chat_settings(),
            MenuAction::SettingsAdvancedGraphics => {
                self.settings_advanced_graphics = !self.settings_advanced_graphics;
            }
            MenuAction::SettingsScale(scale) => {
                self.gui_scale = scale.clamp(1, 4);
                self.set_named_option("gui_scale", i32::from(self.gui_scale));
            }
            MenuAction::SettingsSection(section) => {
                self.settings_section = section;
                self.settings_dropdown = None;
                self.key_remap = None;
            }
            MenuAction::SettingsOption(index, value) => {
                self.set_option(index, value);
                self.settings_dropdown = None;
            }
            MenuAction::SettingsLanguage(index) => self.set_language(index),
            MenuAction::SettingsDropdown(index) => {
                self.settings_dropdown = (self.settings_dropdown != Some(index)).then_some(index);
            }
            MenuAction::SettingsKey(index) => self.key_remap = Some(index),
            MenuAction::SettingsResetKey(index) => {
                if Arc::make_mut(&mut self.settings_options).reset_key(usize::from(index)) {
                    self.settings_dirty = true;
                    self.settings_apply = true;
                } else {
                    self.message =
                        Some("The default key is assigned to another action.".to_owned());
                }
            }
            _ => {}
        }
    }

    /// Captures a desktop key or pointer button for the pending remap operation.
    pub(in crate::menu) fn capture_key(&mut self, control: semantic_input::PhysicalControl) {
        let Some(index) = self.key_remap.take() else {
            return;
        };
        if Arc::make_mut(&mut self.settings_options).remap(usize::from(index), control) {
            self.settings_dirty = true;
            self.settings_apply = true;
        } else {
            self.message = Some("That key is already assigned to another action.".to_owned());
        }
    }
}
