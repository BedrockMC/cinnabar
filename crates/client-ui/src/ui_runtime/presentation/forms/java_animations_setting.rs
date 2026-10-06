//! Video toggle between Java 1.7 and vanilla Bedrock player animations.

use json_ui::Catalog;

use crate::menu::settings_options::JAVA_ANIMATIONS_LABEL;

/// Places the toggle after View Bobbing in the video options.
pub(super) fn install(catalog: &mut Catalog) {
    let overlay = format!(
        r##"{{
  "namespace": "general_section",
  "video_section": {{
    "modifications": [{{
      "array_name": "controls",
      "operation": "insert_after",
      "control_name": "view_bobbing_toggle",
      "value": [{{
        "java_animations@settings_common.option_toggle": {{
          "$option_label": "{JAVA_ANIMATIONS_LABEL}",
          "$option_binding_name": "#java_animations",
          "$option_enabled_binding_name": "#java_animations_enabled",
          "$toggle_name": "java_animations"
        }}
      }}]
    }}]
  }}
}}"##
    );
    catalog.overlay_text("ui/cinnabar_java_animations.json", &overlay);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui_runtime::presentation::forms::pack_harness;

    fn has_text(control: &json_ui::ResolvedControl, text: &str) -> bool {
        control
            .properties
            .get("text")
            .and_then(|value| value.as_str())
            == Some(text)
            || control.children.iter().any(|child| has_text(child, text))
    }

    /// The toggle follows View Bobbing and carries its own caption.
    #[test]
    fn video_options_show_java_animations_after_view_bobbing() {
        let Some(carrier) = pack_harness::carrier() else {
            eprintln!(
                "skipping video_options_show_java_animations_after_view_bobbing: fixture unavailable; requires installed UI carrier (make assets)"
            );
            return;
        };
        let files = carrier.ui_files();
        let mut catalog =
            Catalog::from_files(files.iter().map(|file| (&*file.path, &*file.bytes))).unwrap();
        install(&mut catalog);
        let section = json_ui::resolve(
            &catalog,
            "general_section.video_section",
            &json_ui::Context::retail(false),
        )
        .control
        .expect("video options");
        let position = |name: &str| section.children.iter().position(|child| child.name == name);
        let toggle = position("java_animations").expect("java animations toggle");
        assert_eq!(
            Some(toggle),
            position("view_bobbing_toggle").map(|at| at + 1)
        );
        assert!(has_text(&section.children[toggle], JAVA_ANIMATIONS_LABEL));
    }
}
