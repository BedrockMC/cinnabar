//! Settings-screen values backed by other app resources: the sound section's
//! volume sliders read and write [`AudioSettings`].

use bevy::prelude::ResMut;

use super::MenuRuntime;
use crate::audio::{AudioCategory, AudioSettings};

/// The sound section's mixer categories; text-to-speech persists without a mixer backend.
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
impl MenuRuntime {
    /// Applies the saved sound values to the live mixer.
    pub(crate) fn sync_audio_settings(&mut self, settings: Option<ResMut<AudioSettings>>) {
        let Some(mut settings) = settings else {
            return;
        };
        for (name, category) in VOLUME_SLIDERS {
            if let Some(category) = category {
                let volume = self.settings_options.value(name) as f32 / 100.0;
                if settings.volume(category) != volume {
                    settings.set(category, volume);
                }
            }
        }
    }
}
