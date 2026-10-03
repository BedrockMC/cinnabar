//! Resolved screen flags shared by input and rendering.
use super::{Context, FormEngine};

impl FormEngine {
    /// Reuses the renderer's resolved roots so policy follows the same pack and context.
    pub(in super::super) fn scene_settings(
        &self,
        reference: &str,
        context: &Context,
    ) -> json_ui::ScreenSettings {
        self.screens
            .resolved(reference, &self.catalog, context, || {
                json_ui::resolve(&self.catalog, reference, context).control
            })
            .map_or_else(json_ui::ScreenSettings::default, |root| {
                json_ui::ScreenSettings::from_root(&root)
            })
    }
}
