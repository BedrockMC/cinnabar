//! The vanilla keyboard grid reads the gameplay router's bindings.

use json_ui::{CollectionItem, DataSource, HitRegion, Scalar};

use crate::menu::{
    MenuAction, MenuView,
    settings_options::{KEY_BINDINGS, key_name},
};

/// Populate each supported action's current key and the pack's reset button.
pub(super) fn bind(view: &MenuView, data: &mut DataSource, translate: &dyn Fn(&str) -> String) {
    let rows = KEY_BINDINGS
        .iter()
        .enumerate()
        .map(|(index, (_, label))| {
            let name = translate(label);
            let key = if view.key_remap == Some(index as u16) {
                "...".to_owned()
            } else {
                view.settings_options
                    .key_control(index)
                    .map(key_name)
                    .unwrap_or_default()
            };
            CollectionItem::default()
                .with("#keymapping_name", Scalar::Text(name.clone()))
                .with("#audible_keymapping_name", Scalar::Text(name))
                .with("#binding_button_text", Scalar::Text(key))
        })
        .collect();
    data.set_collection("keyboard_standard_collection", rows);
    data.set_grid_dimensions(
        "#keyboard_standard_grid_dimension",
        [1, KEY_BINDINGS.len() as u32],
    );
}

/// Route collection row presses to capture or restore that action's key.
pub(super) fn action(region: &HitRegion) -> Option<MenuAction> {
    if region.collection.as_deref() != Some("keyboard_standard_collection") {
        return None;
    }
    let index = u16::try_from(region.collection_index?).ok()?;
    match region.pressed.as_deref()? {
        "button.binding_button" => Some(MenuAction::SettingsKey(index)),
        "button.reset_binding" => Some(MenuAction::SettingsResetKey(index)),
        _ => None,
    }
}
