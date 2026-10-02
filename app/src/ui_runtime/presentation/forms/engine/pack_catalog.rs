use super::hud_renderers;
use json_ui::Catalog;

/// Builds the same layered catalog for bootstrap and live reload workers.
pub(in crate::ui_runtime::presentation::forms) fn layer_pack_catalog(
    base: &Catalog,
    layers: &[Vec<(String, Vec<u8>)>],
) -> Catalog {
    let touched = layers
        .iter()
        .flat_map(|files| {
            base.overlay_namespaces(
                files
                    .iter()
                    .map(|(path, bytes)| (path.as_str(), bytes.as_slice())),
            )
        })
        .collect();
    let mut catalog = hud_renderers::with_java_hud(base, &touched);
    for files in layers {
        catalog.apply_pack(
            files
                .iter()
                .map(|(path, bytes)| (path.as_str(), bytes.as_slice())),
        );
    }
    for note in catalog.diagnostics().iter().skip(base.diagnostics().len()) {
        bevy::log::debug!(note, "server ui pack");
    }
    catalog
}
