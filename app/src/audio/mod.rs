//! Positional, category-mixed sound playback driven by packets, local motion and ambience.

mod ambient;
mod bank;
mod echo;
mod engine;
mod local;
mod predicted;
mod route;
mod server;
mod settings;
mod systems;
mod voice;

pub(crate) use bank::{SoundBank, sound_bank_path};
pub(crate) use engine::AudioEngine;
pub(crate) use predicted::LocalBlockCue;
#[cfg(test)]
pub(crate) use server::{SERVER_SOUNDS_TEST_LOCK, current_generation as server_sounds_generation};
pub(crate) use server::{ServerSoundPack, publish_server_sounds};
#[allow(unused_imports)]
pub(crate) use settings::{AudioCategory, AudioSettings};
#[allow(unused_imports)]
pub(crate) use systems::{UiSoundCue, configure, ui_click, ui_sound};
