//! Offline settings gallery through the installed vanilla UI carrier.

use crate::menu::{MenuRuntime, MenuScreen};

struct Metrics;

impl json_ui::TextMeasure for Metrics {
    /// Stable metrics isolate selector geometry from the installed font.
    fn extent(&self, text: &str) -> [f64; 2] {
        [text.len() as f64 * 6.0, 9.0]
    }
}

impl json_ui::TextureSource for Metrics {
    /// Selector dimensions are explicit and do not depend on texture dimensions.
    fn texture(&self, _path: &str) -> Option<json_ui::TextureMeta> {
        None
    }
}

/// Find a resolved selector in the layout, including controls below the viewport.
fn selector(tree: &json_ui::LaidOut<'_>, name: &str) -> Option<json_ui::Rect> {
    if tree.control.name == name {
        return Some(tree.rect);
    }
    tree.children.iter().find_map(|child| selector(child, name))
}

#[test]
fn settings_category_button_pitch_matches_vanilla_toggle_height() {
    let Some(carrier) = super::pack_harness::carrier() else {
        return;
    };
    let files = carrier.ui_files();
    let catalog =
        json_ui::Catalog::from_files(files.iter().map(|file| (&*file.path, &*file.bytes))).unwrap();
    let (reference, context) = super::menu_screens::settings_target();
    let tree = json_ui::resolve(&catalog, reference, &context)
        .control
        .unwrap();
    let layout = json_ui::layout(
        &tree,
        [640.0, 360.0],
        &json_ui::LayoutEnv {
            text: &Metrics,
            textures: &Metrics,
        },
    );
    let general = selector(&layout, "general_button").unwrap();
    let video = selector(&layout, "video_button").unwrap();
    let audio = selector(&layout, "sound_button").unwrap();
    // settings_common.section_toggle_base is 30px; its 31px background overlaps the seam.
    assert_eq!(general.h, 30.0);
    assert_eq!(video.y - general.y, general.h);
    assert_eq!(audio.y - video.y, video.h);
}

/// Render one desktop section selected by its controller variable.
fn section(variable: &str) {
    let mut menu = MenuRuntime::new(true, 2, "Steve".to_owned());
    if variable == "storage_management_forced_index" {
        menu.activate(crate::menu::MenuAction::SettingsSection(
            crate::menu::settings_storage::SECTION_INDEX,
        ));
    }
    let mut view = menu.view();
    let local = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../.local");
    view.language_choices =
        crate::menu::settings_options::SettingsOptions::language_choices(&local);
    view.screen = MenuScreen::Settings;
    view.settings_section = super::menu_screens::SETTINGS_SECTIONS
        .iter()
        .find_map(|(name, index)| (*name == variable).then_some(*index))
        .expect("known settings section");
    super::play_flow_snapshots::snapshot(&view, &format!("settings-{variable}"));
}

#[test]
fn settings_accessibility() {
    section("accessibility_forced_index");
}

#[test]
fn settings_keyboard_mouse() {
    section("keyboard_and_mouse_forced_index");
}

#[test]
fn settings_controller() {
    section("controller_and_switch_forced_index");
}

#[test]
fn settings_general() {
    section("general_forced_index");
}

#[test]
fn settings_profile() {
    section("account_forced_index");
}

#[test]
fn settings_video() {
    section("video_forced_index");
}

#[test]
fn settings_audio() {
    section("sound_forced_index");
}

#[test]
fn settings_creator() {
    section("creator_forced_index");
}

#[test]
fn settings_global_resources() {
    section("global_texture_pack_forced_index");
}

#[test]
fn settings_storage() {
    section("storage_management_forced_index");
}

#[test]
fn settings_language() {
    section("language_forced_index");
}
