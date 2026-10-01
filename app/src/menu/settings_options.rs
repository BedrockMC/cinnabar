//! Persisted legacy JSON-UI option values and their runtime adapters.

mod actions;
mod chat;
mod definitions;
mod keybindings;
mod language;
mod persistence;
mod runtime;
#[cfg(test)]
mod tests;

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

pub(crate) use definitions::{SETTINGS_OPTIONS, SettingDefinition, SettingKind};
pub(crate) use keybindings::{KEY_BINDINGS, key_name};
pub(crate) use persistence::SETTINGS_FILE;

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct SettingsOptions {
    values: BTreeMap<String, i32>,
    keys: BTreeMap<String, u16>,
    language: Option<String>,
    #[serde(skip)]
    startup_gui_scale: Option<u8>,
}

impl SettingsOptions {
    /// Keeps the launch scale visible until the user saves an explicit scale override.
    pub(crate) fn with_gui_scale(mut self, scale: u8) -> Self {
        self.startup_gui_scale = Some(scale.clamp(1, 4));
        self
    }

    /// Preserves the launch scale when no scale override has been saved.
    pub(crate) fn gui_scale(&self, fallback: u8) -> u8 {
        self.values
            .get("gui_scale")
            .map_or(fallback, |value| *value as u8)
    }

    /// Returns the saved value, or the registry default for an untouched option.
    pub(crate) fn get(&self, index: usize) -> i32 {
        let Some(definition) = SETTINGS_OPTIONS.get(index) else {
            return 0;
        };
        self.values
            .get(definition.name)
            .copied()
            .unwrap_or_else(|| {
                if definition.name == "gui_scale" {
                    self.startup_gui_scale.map_or(definition.default, i32::from)
                } else {
                    definition.default
                }
            })
    }

    /// Looks up a runtime setting by its JSON-UI controller name.
    pub(crate) fn value(&self, name: &str) -> i32 {
        SETTINGS_OPTIONS
            .iter()
            .position(|option| option.name == name)
            .map_or(0, |index| self.get(index))
    }

    /// Validates and snaps a UI or persisted value to its declared range.
    pub(crate) fn set(&mut self, index: usize, value: i32) -> bool {
        let Some(definition) = SETTINGS_OPTIONS.get(index) else {
            return false;
        };
        let value = value.clamp(definition.min, definition.max);
        let value = definition.min + ((value - definition.min) / definition.step) * definition.step;
        if self.get(index) == value {
            return false;
        }
        self.values.insert(definition.name.to_owned(), value);
        true
    }
}
