//! Reset groups follow the controls authored in general_section.json.

use super::{SETTINGS_OPTIONS, SettingsOptions};
use crate::menu::{MenuDialog, MenuRuntime};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SettingsGroup {
    Video,
    Accessibility,
    Audio,
}

impl SettingsGroup {
    /// Limits resets to registered controls in the authored section, including shared controls.
    fn contains(self, name: &str) -> bool {
        match self {
            // P:general_section.json:3000–4058; max_framerate is an inherited slider.
            Self::Video => matches!(
                name,
                "graphics_mode"
                    | "render_distance"
                    | "gamma"
                    | "third_person"
                    | "full_screen"
                    | "hide_hand"
                    | "hide_paperdoll"
                    | "hide_hud"
                    | "screen_animations"
                    | "panorama_speed"
                    | "interface_opacity"
                    | "field_of_view"
                    | "show_auto_save_icon"
                    | "classic_box_selection"
                    | "ingame_player_names"
                    | "view_bobbing"
                    | "camera_shake"
                    | "transparent_leaves"
                    | "bubble_particles"
                    | "render_clouds"
                    | "fancy_skies"
                    | "smooth_lighting"
                    | "field_of_view_toggle"
                    | "damage_bob"
                    | "gui_scale"
                    | "gui_accessibility_scaling"
                    | "max_framerate"
            ),
            // P:general_section.json:4738–5158.
            Self::Accessibility => matches!(
                name,
                "enable_gameplay_subtitles"
                    | "hide_own_gameplay_subtitles"
                    | "hide_ambient_gameplay_subtitles"
                    | "enable_ui_text_to_speech"
                    | "enable_chat_text_to_speech"
                    | "texttospeech_volume"
                    | "enable_open_chat_message"
                    | "hud_text_background_opacity"
                    | "chat_background_opacity"
                    | "actionbar_text_background_opacity"
                    | "camera_shake"
                    | "hide_endflash"
                    | "enable_dithering_blocks"
                    | "enable_dithering_mobs"
                    | "darkness"
                    | "screen_distortion"
                    | "glint_strength"
                    | "glint_speed"
                    | "toast_notification_duration"
                    | "chat_message_duration"
                    | "gui_scale"
                    | "gui_accessibility_scaling"
            ),
            // P:general_section.json:5174–5506; mixer bindings have one shared registry.
            Self::Audio => crate::menu::settings_values::VOLUME_SLIDERS
                .iter()
                .any(|(volume, _)| *volume == name),
        }
    }
}

impl SettingsOptions {
    /// Resets only one section to registry defaults, preserving bindings and other sections.
    fn reset_group(&mut self, group: SettingsGroup) -> bool {
        let mut changed = false;
        for (index, option) in SETTINGS_OPTIONS.iter().enumerate() {
            if group.contains(option.name) {
                changed |= self.set(index, option.default);
            }
        }
        changed
    }
}

impl MenuRuntime {
    /// Applies a section reset only after its matching confirmation remains open.
    pub(in crate::menu) fn confirm_settings_reset(&mut self, group: SettingsGroup) {
        if self.dialog != Some(MenuDialog::SettingsResetGroup(group)) {
            return;
        }
        self.dialog = None;
        if std::sync::Arc::make_mut(&mut self.settings_options).reset_group(group) {
            self.gui_scale = self.settings_options.value("gui_scale") as u8;
            self.settings_dirty = true;
            self.settings_apply = true;
        }
        self.settings_dropdown = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::menu::{MenuAction, MenuRuntime};

    /// Changes a registry option without introducing another copy of its default.
    fn change(options: &mut SettingsOptions, name: &str) -> i32 {
        let index = SETTINGS_OPTIONS
            .iter()
            .position(|option| option.name == name)
            .unwrap();
        let option = SETTINGS_OPTIONS[index];
        let value = if option.default == option.min {
            option.max
        } else {
            option.min
        };
        assert!(options.set(index, value));
        value
    }

    #[test]
    fn section_reset_restores_defaults_and_preserves_unrelated_settings() {
        for (group, member, unrelated) in [
            (SettingsGroup::Video, "field_of_view", "main_volume"),
            (
                SettingsGroup::Accessibility,
                "chat_background_opacity",
                "field_of_view",
            ),
            (
                SettingsGroup::Audio,
                "main_volume",
                "chat_background_opacity",
            ),
        ] {
            let mut options = SettingsOptions::default();
            change(&mut options, member);
            let retained = change(&mut options, unrelated);
            let key = options.key_control(0);
            assert!(options.reset_group(group));
            let default = SETTINGS_OPTIONS
                .iter()
                .find(|option| option.name == member)
                .unwrap()
                .default;
            assert_eq!(options.value(member), default);
            assert_eq!(options.value(unrelated), retained);
            assert_eq!(options.key_control(0), key);
        }
    }

    #[test]
    fn reset_cancel_and_mismatched_confirmation_preserve_values() {
        let mut menu = MenuRuntime::new(true, 2, "Steve".into());
        let value = change(
            std::sync::Arc::make_mut(&mut menu.settings_options),
            "field_of_view",
        );
        menu.activate(MenuAction::SettingsResetGroup(SettingsGroup::Video));
        assert_eq!(menu.settings_options.value("field_of_view"), value);
        menu.activate(MenuAction::SettingsConfirmResetGroup(SettingsGroup::Audio));
        assert_eq!(menu.settings_options.value("field_of_view"), value);
        menu.activate(MenuAction::DismissDialog);
        menu.activate(MenuAction::SettingsConfirmResetGroup(SettingsGroup::Video));
        assert_eq!(menu.settings_options.value("field_of_view"), value);
        menu.activate(MenuAction::SettingsResetGroup(SettingsGroup::Video));
        menu.activate(MenuAction::SettingsConfirmResetGroup(SettingsGroup::Video));
        assert!(menu.dialog.is_none());
        assert_eq!(
            menu.settings_options.value("field_of_view"),
            SettingsOptions::default().value("field_of_view")
        );
        assert!(menu.settings_dirty && menu.settings_apply);
    }
}
