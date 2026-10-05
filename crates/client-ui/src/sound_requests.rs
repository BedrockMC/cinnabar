//! Bounded sound requests drained by the app audio adapter at its existing frame stage.

use std::sync::atomic::{AtomicU32, Ordering};

static PENDING_UI_CLICKS: AtomicU32 = AtomicU32::new(0);

/// Requests the interface click sound from any code path (no ECS access needed); coalesced per frame.
pub fn ui_click() {
    let _ = PENDING_UI_CLICKS.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |count| {
        Some(count.saturating_add(1))
    });
}

/// Interface sounds JSON-UI sound components asked for: name, volume, pitch.
static PENDING_UI_SOUNDS: std::sync::Mutex<Vec<(String, f32, f32)>> =
    std::sync::Mutex::new(Vec::new());
/// Bounds one frame's queued control sounds.
const MAX_PENDING_UI_SOUNDS: usize = 16;

/// Plays a pressed launcher control's sound, holding back a repeat inside its
/// `min_seconds_between_plays`, as vanilla's sound component does.
pub fn ui_control_sound(sound: &json_ui::ControlSound) {
    static LAST_PLAYED: std::sync::Mutex<Vec<(String, std::time::Instant)>> =
        std::sync::Mutex::new(Vec::new());
    if sound.min_seconds > 0.0 {
        let mut played = LAST_PLAYED
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let now = std::time::Instant::now();
        match played.iter().position(|(name, _)| *name == sound.name) {
            Some(index)
                if now.duration_since(played[index].1).as_secs_f32() < sound.min_seconds =>
            {
                return;
            }
            Some(index) => played[index].1 = now,
            None if played.len() < MAX_PENDING_UI_SOUNDS => played.push((sound.name.clone(), now)),
            None => {}
        }
    }
    ui_sound(&sound.name, sound.volume, sound.pitch);
}

/// Requests an interface sound a UI sound component names, at its volume and pitch.
pub fn ui_sound(name: &str, volume: f32, pitch: f32) {
    let mut pending = PENDING_UI_SOUNDS
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    if pending.len() < MAX_PENDING_UI_SOUNDS {
        pending.push((name.to_owned(), volume, pitch));
    }
}

/// Takes the coalesced click request at the existing audio pump boundary.
pub fn take_click() -> bool {
    PENDING_UI_CLICKS.swap(0, Ordering::Relaxed) > 0
}

/// Takes the queued named sounds in their original request order.
pub fn take_sounds() -> Vec<(String, f32, f32)> {
    std::mem::take(
        &mut *PENDING_UI_SOUNDS
            .lock()
            .unwrap_or_else(|poison| poison.into_inner()),
    )
}
