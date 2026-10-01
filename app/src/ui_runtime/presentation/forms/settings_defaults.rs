//! Values the settings screen shows for options the client does not back yet:
//! the vanilla desktop defaults, with each option left disabled (its `_enabled`
//! binding unset) so it reads as unavailable rather than silently ignoring input.

use json_ui::{DataSource, Scalar};

/// Dropdowns: the name its bindings share, the default radio's binding and
/// label key.
const DROPDOWNS: &[(&str, &str, &str)] = &[
    (
        "graphics_mode",
        "#graphics_mode_radio_fancy",
        "options.graphicsMode.fancy",
    ),
    (
        "third_person",
        "#thirdperson_radio_first",
        "options.thirdperson.firstperson",
    ),
    (
        "split_screen",
        "#split_screen_radio_horizontal",
        "options.splitscreen.horizontal",
    ),
    (
        "ui_profile",
        "#ui_profile_radio_classic",
        "options.uiprofile.classic",
    ),
];

/// Sliders: the name its bindings share, its label key, the default position
/// (0..=1) and how the value reads.
const SLIDERS: &[(&str, &str, f64, &str)] = &[
    ("gamma", "options.gamma", 0.5, "50%"),
    ("interface_opacity", "options.hudOpacity", 1.0, "100%"),
    (
        "splitscreen_interface_opacity",
        "options.splitscreenInterfaceOpacity",
        1.0,
        "100%",
    ),
    // Bedrock's FOV runs 30..=110 and starts at 60.
    ("field_of_view", "options.fov", 0.375, "60"),
    ("damage_bob", "options.damageBobbing", 1.0, "100%"),
];

/// Toggles on by default; every other toggle reads off.
const TOGGLES_ON: &[&str] = &[
    "#transparent_leaves",
    "#bubble_particles",
    "#render_clouds",
    "#fancy_skies",
    "#smooth_lighting",
    "#view_bobbing",
    "#camera_shake",
    "#ingame_player_names",
    "#splitscreen_ingame_player_names",
    "#show_auto_save_icon",
];

/// Bind the vanilla defaults of the options the client does not back.
pub(super) fn bind(data: &mut DataSource, translate: &dyn Fn(&str) -> String) {
    for (name, radio, label) in DROPDOWNS {
        data.set_global(
            format!("#{name}_dropdown_toggle_label"),
            Scalar::Text(translate(label)),
        );
        data.set_global(*radio, Scalar::Bool(true));
    }
    for (name, label, position, value) in SLIDERS {
        data.set_global(format!("#{name}"), Scalar::Num(*position));
        data.set_global(
            format!("#{name}_slider_label"),
            // The label localizes again, where `%%` keeps one `%`.
            Scalar::Text(format!(
                "{}: {}",
                translate(label),
                value.replace('%', "%%")
            )),
        );
    }
    for toggle in TOGGLES_ON {
        data.set_global(*toggle, Scalar::Bool(true));
    }
    // With no global packs, `ResourcePacksScreenController`'s cycling icon falls
    // back to the vanilla pack's (`ResourcePack::getIconPath`).
    data.set_global(
        "#cycling_icon_path_global",
        Scalar::Text(format!(
            "{}pack_icon.png",
            super::server_pack::VANILLA_IN_PACKAGE
        )),
    );
}
