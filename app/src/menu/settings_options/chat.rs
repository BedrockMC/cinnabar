//! Chat settings share the persisted option registry with the main settings screen.

use std::sync::Arc;

use super::definitions::SettingChoice;
use super::{SETTINGS_OPTIONS, SettingsOptions};
use crate::menu::MenuRuntime;

impl MenuRuntime {
    /// Supplies a cheap settings snapshot even while the launcher menu is hidden.
    pub(crate) fn settings_snapshot(&self) -> (Arc<SettingsOptions>, Option<u16>) {
        (Arc::clone(&self.settings_options), self.settings_dropdown)
    }

    /// Restores the chat popup's registered options without changing other sections.
    pub(in crate::menu) fn reset_chat_settings(&mut self) {
        for (index, option) in SETTINGS_OPTIONS.iter().enumerate() {
            if matches!(
                option.name,
                "hide_chat"
                    | "toggle_emote_chat"
                    | "toggle_tts"
                    | "chat_typeface"
                    | "chat_font_size"
                    | "chat_line_spacing"
                    | "chat_color"
                    | "mentions_color"
            ) {
                self.set_option(index as u16, option.default);
            }
        }
    }
}

pub(super) const TYPEFACES: &[SettingChoice] = &[
    SettingChoice {
        name: "typeface_radio_mojangles",
        label: "typeface.mojangles",
    },
    SettingChoice {
        name: "typeface_radio_notoSans",
        label: "typeface.notoSans",
    },
];
pub(super) const CHAT_COLORS: &[SettingChoice] = &[
    SettingChoice {
        name: "chat_0",
        label: "color.white",
    },
    SettingChoice {
        name: "chat_1",
        label: "color.green",
    },
    SettingChoice {
        name: "chat_2",
        label: "color.aqua",
    },
    SettingChoice {
        name: "chat_3",
        label: "color.red",
    },
    SettingChoice {
        name: "chat_4",
        label: "color.light_purple",
    },
    SettingChoice {
        name: "chat_5",
        label: "color.yellow",
    },
    SettingChoice {
        name: "chat_6",
        label: "color.gold",
    },
];
pub(super) const MENTIONS_COLORS: &[SettingChoice] = &[
    SettingChoice {
        name: "mentions_0",
        label: "color.white",
    },
    SettingChoice {
        name: "mentions_1",
        label: "color.green",
    },
    SettingChoice {
        name: "mentions_2",
        label: "color.aqua",
    },
    SettingChoice {
        name: "mentions_3",
        label: "color.red",
    },
    SettingChoice {
        name: "mentions_4",
        label: "color.light_purple",
    },
    SettingChoice {
        name: "mentions_5",
        label: "color.yellow",
    },
    SettingChoice {
        name: "mentions_6",
        label: "color.gold",
    },
];

impl SettingsOptions {
    /// Mirrors ChatUtils::canLanguageBeSmooth's four unsupported locales.
    pub(crate) fn chat_smooth_available(&self) -> bool {
        !matches!(self.language(), Some("zh_TW" | "zh_CN" | "ko_KR" | "ja_JP"))
    }

    /// Scales the open chat's text; the exact retail font-size option range remains unverified.
    pub(crate) fn chat_font_scale(&self) -> f64 {
        if !self.chat_smooth_available() || self.value("chat_typeface") == 0 {
            1.0
        } else {
            f64::from(self.value("chat_font_size")) / 10.0
        }
    }

    /// Applies ChatUtils' one-decimal padding plus the source's nonzero epsilon.
    pub(crate) fn chat_line_padding(&self) -> f64 {
        f64::from(self.value("chat_line_spacing")) / 10.0 + 0.001
    }

    /// Uses the seven legacy chat colors from the reconstructed controller's indexed palette.
    pub(crate) fn chat_color_code(&self) -> char {
        ['f', 'a', 'b', 'c', 'd', 'e', '6'][self.value("chat_color") as usize]
    }

    /// Matches the three authored notification-duration radio choices.
    pub(crate) fn chat_lifetime(&self) -> f64 {
        notification_millis(self.value("chat_message_duration")) as f64 / 1_000.0
    }

    /// Uses the pack's three toast-duration choices for new notification requests.
    pub(crate) fn toast_lifetime_millis(&self) -> u64 {
        notification_millis(self.value("toast_notification_duration"))
    }
}

/// Both notification menus author the same three duration choices.
fn notification_millis(index: i32) -> u64 {
    match index {
        1 => 10_000,
        2 => 30_000,
        _ => ui::TOAST_DISPLAY_MILLIS,
    }
}
