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

    #[test]
    fn only_backed_sliders_accept_changes() {
        let mut menu = MenuRuntime::new(true, 2, "Steve".to_owned());
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
