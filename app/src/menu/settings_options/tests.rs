use super::*;
use semantic_input::{InputContext, PhysicalControl};

/// Finds an option through the same stable controller identifier used on disk.
fn index(name: &str) -> usize {
    SETTINGS_OPTIONS
        .iter()
        .position(|option| option.name == name)
        .unwrap()
}

#[test]
fn all_option_families_round_trip_and_reject_unknown_fields() {
    let mut settings = SettingsOptions::default();
    for (index, definition) in SETTINGS_OPTIONS.iter().enumerate() {
        assert_eq!(settings.get(index), definition.default);
        assert!(definition.min <= definition.default && definition.default <= definition.max);
        settings.set(index, definition.min);
    }
    let bytes = serde_json::to_vec(&settings).unwrap();
    assert_eq!(SettingsOptions::decode(&bytes).unwrap(), settings);
    let loaded =
        SettingsOptions::decode(br#"{"values":{"gamma":500,"field_of_view":-20,"unknown":100}}"#)
            .unwrap();
    assert_eq!(loaded.value("gamma"), SETTINGS_OPTIONS[index("gamma")].max);
    assert_eq!(
        loaded.value("field_of_view"),
        SETTINGS_OPTIONS[index("field_of_view")].min
    );
    assert!(!loaded.values.contains_key("unknown"));
    assert!(SettingsOptions::decode(b"broken").is_none());
}

#[test]
fn camera_input_and_window_settings_read_the_saved_values() {
    let mut settings = SettingsOptions::default();
    for (name, value) in [
        ("field_of_view", 82),
        ("full_screen", 1),
        ("view_bobbing", 0),
        ("field_of_view_toggle", 0),
        ("camera_shake", 0),
        ("damage_bob", 25),
        ("keyboard_mouse_sensitivity", 75),
        ("keyboard_mouse_invert_y_axis", 1),
        ("third_person", 2),
        ("max_framerate", 120),
    ] {
        settings.set(index(name), value);
    }
    let settings = SettingsOptions::decode(&serde_json::to_vec(&settings).unwrap()).unwrap();
    let user = settings.user_settings();
    let mut authority = crate::camera::CameraSettingsAuthority::default();
    authority.replace(1, &user).unwrap();
    assert_eq!(authority.horizontal_fov_degrees(), 82.0);
    assert!(!authority.feel().view_bobbing);
    assert!(!authority.feel().camera_shake);
    assert_eq!(authority.feel().damage_bob, 0.25);
    assert_eq!(authority.feel().fov_effects_scale, 0.0);
    assert_eq!(user.controls.mouse_sensitivity, 1.5);
    assert!(user.controls.invert_mouse_y);
    assert_eq!(user.video.frame_cap, Some(120));
    assert!(user.video.fullscreen);
    assert_eq!(
        authority.perspective(),
        semantic_input::PerspectiveMode::ThirdPersonFront
    );
}

#[test]
fn remapped_keyboard_controls_reach_gameplay_and_survive_reload() {
    let mut settings = SettingsOptions::default();
    let index = KEY_BINDINGS
        .iter()
        .position(|(_, name)| *name == "key.forward")
        .unwrap();
    let new_key = PhysicalControl::KeyboardUsage(0x0c);
    assert!(settings.remap(index, new_key));
    let restored = SettingsOptions::decode(&serde_json::to_vec(&settings).unwrap()).unwrap();
    assert_eq!(restored.key_control(index), Some(new_key));
    let controls = restored.user_settings().controls;
    assert!(controls.bindings().iter().any(|binding| binding.action
        == semantic_input::Action::MoveForward
        && binding.context == InputContext::Gameplay
        && binding.chord.control == new_key));
    assert!(!settings.remap(index, PhysicalControl::KeyboardUsage(0x16)));
    settings.reset_key(index);
    assert_eq!(
        settings.key_control(index),
        SettingsOptions::default().key_control(index)
    );
}

#[test]
fn saved_volumes_reach_the_mixer() {
    use crate::{
        audio::{AudioCategory, AudioSettings},
        menu::MenuRuntime,
    };
    use bevy::prelude::{App, ResMut, Update};
    /// Exercises the production sound adapter without writing settings to disk.
    fn sync(mut menu: ResMut<MenuRuntime>, audio: ResMut<AudioSettings>) {
        menu.sync_audio_settings(Some(audio));
    }
    let mut menu = MenuRuntime::new(true, 2, "Settings test".to_owned());
    menu.set_option(index("main_volume") as u16, 50);
    menu.set_option(index("music_volume") as u16, 40);
    let mut app = App::new();
    app.insert_resource(menu)
        .init_resource::<AudioSettings>()
        .add_systems(Update, sync);
    app.update();
    let mixer = app.world().resource::<AudioSettings>();
    assert!((mixer.effective(AudioCategory::Music) - 0.2).abs() < 0.0001);
    assert_eq!(mixer.volume(AudioCategory::Master), 0.5);
}

#[test]
fn settings_file_replacement_round_trips() {
    let directory = std::env::temp_dir().join(format!("cinnabar-settings-{}", std::process::id()));
    let path = directory.join(SETTINGS_FILE);
    let mut settings = SettingsOptions::default();
    settings.set(index("gamma"), 80);
    settings.save(&path).unwrap();
    assert_eq!(SettingsOptions::load(&path), settings);
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn reset_conflict_preserves_every_saved_mapping() {
    let mut settings = SettingsOptions::default();
    let forward = KEY_BINDINGS
        .iter()
        .position(|(_, name)| *name == "key.forward")
        .unwrap();
    let backward = KEY_BINDINGS
        .iter()
        .position(|(_, name)| *name == "key.back")
        .unwrap();
    let default_forward = settings.key_control(forward).unwrap();
    assert!(settings.remap(forward, PhysicalControl::KeyboardUsage(0x0c)));
    assert!(settings.remap(backward, default_forward));
    let before = settings.clone();
    assert!(!settings.reset_key(forward));
    assert_eq!(settings, before);
}

#[test]
fn launch_scale_matches_the_snapshot_without_persisting_an_unedited_override() {
    let mut settings = SettingsOptions::default().with_gui_scale(3);
    let gui = index("gui_scale");
    assert_eq!(settings.get(gui), 3);
    assert!(!settings.values.contains_key("gui_scale"));
    settings.set(gui, 2);
    let loaded = SettingsOptions::decode(&serde_json::to_vec(&settings).unwrap())
        .unwrap()
        .with_gui_scale(3);
    assert_eq!(loaded.get(gui), 2);
}

#[test]
fn chat_settings_survive_reload_and_feed_the_chat_renderer() {
    let mut settings = SettingsOptions::default();
    for (name, value) in [
        ("chat_typeface", 1),
        ("chat_font_size", 15),
        ("chat_line_spacing", 25),
        ("chat_color", 3),
        ("chat_message_duration", 2),
    ] {
        settings.set(index(name), value);
    }
    let restored = SettingsOptions::decode(&serde_json::to_vec(&settings).unwrap()).unwrap();
    assert_eq!(restored.chat_font_scale(), 1.5);
    assert_eq!(restored.chat_line_padding(), 2.501);
    assert_eq!(restored.chat_color_code(), 'c');
    assert_eq!(restored.chat_lifetime(), 30.0);
}
