//! Per-category volume sliders and the sound-definition category they gate.

use bevy::prelude::Resource;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum AudioCategory {
    Master,
    Music,
    Sound,
    Ambient,
    Players,
    Blocks,
    Hostile,
    Neutral,
    Records,
    Weather,
    Ui,
}

impl AudioCategory {
    const COUNT: usize = 11;

    /// Slider bucket of a sound definition's `category`; unknown names use the generic sound bucket.
    pub(crate) fn from_definition(category: Option<&str>) -> Self {
        match category {
            Some("music") => Self::Music,
            Some("ambient") => Self::Ambient,
            Some("player") => Self::Players,
            Some("block") => Self::Blocks,
            Some("hostile") => Self::Hostile,
            Some("neutral") => Self::Neutral,
            Some("record") => Self::Records,
            Some("weather") => Self::Weather,
            Some("ui") => Self::Ui,
            _ => Self::Sound,
        }
    }

    const fn slot(self) -> usize {
        self as usize
    }
}

/// Volume sliders in `[0, 1]`; the master slider scales every other category.
#[derive(Resource, Clone, Debug, PartialEq)]
pub(crate) struct AudioSettings {
    volumes: [f32; AudioCategory::COUNT],
    /// Skips definitions marked `load_on_low_memory: false`.
    pub(crate) low_memory: bool,
}

impl Default for AudioSettings {
    fn default() -> Self {
        Self {
            volumes: [1.0; AudioCategory::COUNT],
            low_memory: false,
        }
    }
}

impl AudioSettings {
    pub(crate) fn set(&mut self, category: AudioCategory, volume: f32) {
        if volume.is_finite() {
            self.volumes[category.slot()] = volume.clamp(0.0, 1.0);
        }
    }

    /// Effective gain of `category`: its own slider times master (master alone for `Master`).
    pub(crate) fn effective(&self, category: AudioCategory) -> f32 {
        let master = self.volumes[AudioCategory::Master.slot()];
        if category == AudioCategory::Master {
            master
        } else {
            master * self.volumes[category.slot()]
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn master_scales_every_category() {
        let mut settings = AudioSettings::default();
        settings.set(AudioCategory::Master, 0.5);
        settings.set(AudioCategory::Music, 0.4);
        assert!((settings.effective(AudioCategory::Music) - 0.2).abs() < 1e-6);
        assert!((settings.effective(AudioCategory::Blocks) - 0.5).abs() < 1e-6);
        settings.set(AudioCategory::Ui, f32::NAN);
        assert_eq!(settings.effective(AudioCategory::Ui), 0.5);
    }

    #[test]
    fn definition_categories_map_to_buckets() {
        assert_eq!(
            AudioCategory::from_definition(Some("record")),
            AudioCategory::Records
        );
        assert_eq!(
            AudioCategory::from_definition(Some("bottle")),
            AudioCategory::Sound
        );
        assert_eq!(AudioCategory::from_definition(None), AudioCategory::Sound);
    }
}
