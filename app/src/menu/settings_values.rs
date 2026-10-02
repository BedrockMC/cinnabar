//! Settings-screen values backed by other app resources: the sound section's
//! volume sliders read and write [`AudioSettings`].

use bevy::prelude::ResMut;

use super::MenuRuntime;
use crate::audio::{AudioCategory, AudioSettings};

/// The sound section's sliders in screen order with the category each sets;
/// text-to-speech has no app category and stays disabled.
pub(crate) const VOLUME_SLIDERS: [(&str, Option<AudioCategory>); 11] = [
    ("main_volume", Some(AudioCategory::Master)),
    ("music_volume", Some(AudioCategory::Music)),
    ("sound_volume", Some(AudioCategory::Sound)),
    ("ambient_volume", Some(AudioCategory::Ambient)),
    ("block_volume", Some(AudioCategory::Blocks)),
    ("hostile_volume", Some(AudioCategory::Hostile)),
    ("neutral_volume", Some(AudioCategory::Neutral)),
    ("player_volume", Some(AudioCategory::Players)),
    ("record_volume", Some(AudioCategory::Records)),
    ("weather_volume", Some(AudioCategory::Weather)),
    ("texttospeech_volume", None),
];
/// Positions a volume slider snaps to, 0% to 100%; granularity needs native measurement.
pub(crate) const VOLUME_STEPS: u8 = 21;

/// Slider percents in [`VOLUME_SLIDERS`] order; `None` is an unbacked slider.
pub(crate) type Volumes = [Option<u8>; VOLUME_SLIDERS.len()];

impl MenuRuntime {
    /// A capture's fixed CLI scale, cleared when the native slider is changed.
    pub(crate) fn gui_scale_preference(&self) -> Option<u8> {
        self.gui_scale_preference
    }

    #[cfg(test)]
    pub(crate) fn set_gui_scale_preference(&mut self, preference: Option<u8>) {
        self.gui_scale_preference = preference
            .filter(|scale| *scale > 0)
            .map(|scale| scale.clamp(1, 4));
    }

    pub(crate) fn gui_scale_offset(&self) -> i8 {
        self.gui_scale_offset
    }

    pub(super) fn set_gui_scale_offset(&mut self, offset: i8) {
        if self.gui_scale_choices.contains(&offset) {
            self.gui_scale_preference = None;
            self.gui_scale_offset = offset;
            self.gui_scale_display_offset = offset;
        }
    }

    /// The native choices track the physical viewport; the saved modifier
    /// survives resize and is clamped when the rendering scale is evaluated.
    pub(crate) fn sync_gui_scale(&mut self, displayed_offset: i8, choices: Vec<i8>) {
        self.gui_scale_display_offset = displayed_offset.clamp(
            choices.first().copied().unwrap_or(0),
            choices.last().copied().unwrap_or(0),
        );
        if self.gui_scale_choices != choices {
            self.gui_scale_choices = choices;
        }
    }

    /// A settings press waiting to be applied to the primary window.
    pub(crate) fn take_fullscreen_change(&mut self) -> Option<bool> {
        self.fullscreen_change.take()
    }

    /// Mirror the window without queuing a new settings press.
    pub(crate) fn sync_fullscreen(&mut self, fullscreen: bool) {
        self.fullscreen = fullscreen;
    }

    /// Write a pending slider change into `settings`, then mirror its sliders.
    pub(crate) fn sync_audio_settings(&mut self, settings: Option<ResMut<AudioSettings>>) {
        let Some(mut settings) = settings else {
            return;
        };
        if let Some((slot, percent)) = self.volume_change.take()
            && let Some((_, Some(category))) = VOLUME_SLIDERS.get(usize::from(slot))
        {
            settings.set(*category, f32::from(percent) / 100.0);
        }
        self.volumes = VOLUME_SLIDERS.map(|(_, category)| {
            category.map(|category| (settings.volume(category) * 100.0).round() as u8)
        });
    }

    pub(super) fn set_volume(&mut self, slot: u8, percent: u8) {
        let percent = percent.min(100);
        if let Some(Some(volume)) = self.volumes.get_mut(usize::from(slot)) {
            *volume = percent;
            self.volume_change = Some((slot, percent));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn menu() -> MenuRuntime {
        let mut menu = MenuRuntime::new(true, 2, "Steve".to_owned());
        // Saved host settings are exercised by the dedicated persistence tests.
        menu.sync_fullscreen(false);
        let _ = menu.take_fullscreen_change();
        menu
    }

    #[test]
    fn native_gui_scale_choice_clears_the_fixed_cli_override() {
        let mut menu = menu();
        menu.set_gui_scale_preference(Some(2));
        menu.sync_gui_scale(0, vec![-1, 0]);
        menu.activate(super::super::MenuAction::SettingsScale(-1));
        assert_eq!(menu.gui_scale_preference(), None);
        assert_eq!(menu.gui_scale_offset(), -1);
    }

    #[test]
    fn fullscreen_mirroring_does_not_queue_a_setting_change() {
        let mut menu = menu();
        menu.sync_fullscreen(true);
        assert!(menu.view().fullscreen);
        assert_eq!(menu.take_fullscreen_change(), None);
        menu.activate(super::super::MenuAction::SettingsFullscreen(false));
        assert!(!menu.view().fullscreen);
        assert_eq!(menu.take_fullscreen_change(), Some(false));
        assert_eq!(menu.take_fullscreen_change(), None);
    }

    #[test]
    fn only_backed_sliders_accept_changes() {
        let mut menu = menu();
        menu.set_volume(1, 40);
        assert_eq!(
            menu.view().volumes[1],
            None,
            "unsynced sliders are unbacked"
        );
        menu.volumes[1] = Some(100);
        menu.set_volume(1, 40);
        menu.set_volume(10, 40);
        assert_eq!(menu.volume_change, Some((1, 40)));
        assert_eq!(menu.view().volumes[1], Some(40));
    }
}
