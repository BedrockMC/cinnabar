use bevy::prelude::Resource;
use ui::UserSettings;

/// App-owned retained settings handoff used by menus and live subsystem
/// adapters. Every replacement is complete and monotonically versioned.
#[derive(Resource, Clone, Debug, Default)]
pub struct RuntimeSettings {
    generation: u64,
    user_settings: UserSettings,
}

impl RuntimeSettings {
    /// Write back the fullscreen adapter's already-applied window state without
    /// publishing a complete settings replacement. A window toggle must not
    /// apply unrelated camera or VSync defaults through their generation readers.
    pub(crate) fn set_fullscreen(&mut self, fullscreen: bool) {
        self.user_settings.video.fullscreen = fullscreen;
    }

    pub fn replace_user_settings(&mut self, settings: UserSettings) -> u64 {
        self.generation = self.generation.saturating_add(1);
        self.user_settings = settings;
        self.generation
    }

    #[must_use]
    pub const fn user_settings_update(&self) -> (u64, &UserSettings) {
        (self.generation, &self.user_settings)
    }
}
