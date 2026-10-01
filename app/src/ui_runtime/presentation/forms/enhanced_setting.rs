//! The optional Video-section extension, separate from vanilla settings wiring.

use json_ui::{Catalog, DataSource, HitRegion, Scalar};

use crate::menu::{MenuAction, MenuScreen, MenuView};

/// Add one control using the existing JSON-UI option template.
pub(super) fn install(catalog: &mut Catalog) {
    catalog.apply_pack([("ui/cinnabar_enhanced.json", OVERLAY)]);
}

/// Publish the extension toggle without changing vanilla option bindings.
pub(super) fn bind(view: &MenuView, data: &mut DataSource) {
    data.set_global(
        "#cinnabar_enhanced",
        Scalar::Bool(view.render_mode == ui::RenderMode::Enhanced),
    );
    data.set_global("#cinnabar_enhanced_enabled", Scalar::Bool(true));
}

/// Route only the extension control to its retained setting request.
pub(super) fn action(view: &MenuView, region: &HitRegion) -> Option<MenuAction> {
    (view.screen == MenuScreen::Settings
        && region.control_name.as_deref() == Some("cinnabar_enhanced"))
    .then_some(MenuAction::ToggleRenderMode)
}

const OVERLAY: &[u8] = br##"{
  "namespace": "general_section",
  "video_section": {
    "modifications": [{
      "array_name": "controls",
      "operation": "insert_front",
      "value": [{
        "cinnabar_enhanced@settings_common.option_toggle": {
          "$option_label": "Enhanced rendering (Cinnabar extension)",
          "$option_binding_name": "#cinnabar_enhanced",
          "$option_enabled_binding_name": "#cinnabar_enhanced_enabled",
          "$toggle_name": "cinnabar_enhanced"
        }
      }]
    }]
  }
}"##;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extension_overlay_adds_one_video_control() {
        let mut catalog = Catalog::default();
        catalog.overlay_text(
            "ui/general_section.json",
            r#"{"namespace":"general_section","video_section":{"type":"stack_panel","controls":[]}}"#,
        );
        catalog.overlay_text(
            "ui/settings_common.json",
            r#"{"namespace":"settings_common","option_toggle":{"type":"toggle"}}"#,
        );
        install(&mut catalog);
        assert!(catalog.diagnostics().is_empty());
        let resolution = json_ui::resolve(
            &catalog,
            "general_section.video_section",
            &json_ui::Context::default(),
        );
        assert!(resolution.diagnostics.is_empty());
        let resolved = resolution.control.expect("video section");
        assert_eq!(resolved.children.len(), 1);
        assert_eq!(resolved.children[0].name, "cinnabar_enhanced");
        assert_eq!(resolved.children[0].control_type.as_deref(), Some("toggle"));
    }
}
