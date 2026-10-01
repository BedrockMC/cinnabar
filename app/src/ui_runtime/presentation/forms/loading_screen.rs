//! Joining a world through vanilla's world-loading progress screen for the
//! dimension (dirt, netherrack or end-stone backdrop): "Locating server" until
//! the world starts, then "Building terrain" until the first view settles.

use std::sync::Arc;

use json_ui::{DataSource, Scalar, ViewState};
use ui::UiNode;

use super::super::{
    FONT_DESIGN_PIXEL_TEXELS, TextMetrics, UiPresentationError, UiPresentationRuntime,
};
use super::engine::{EngineInputs, EngineOutput, ScreenArt};
use crate::ui_runtime::UiRuntime;

/// Which part of joining the loading screen reports.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LoadingStage {
    Connecting,
    BuildingTerrain,
}

impl UiPresentationRuntime {
    /// Draw the world-loading screen for `stage`; `Ok(false)` when nothing drew.
    pub(in super::super) fn append_loading_screen(
        &mut self,
        runtime: &UiRuntime,
        stage: LoadingStage,
        nodes: &mut Vec<UiNode>,
        next: &mut u32,
        metrics: TextMetrics,
        content: [f32; 2],
    ) -> Result<bool, UiPresentationError> {
        let Some(renderer) = self.form_presentation.engine.as_deref() else {
            return Ok(false);
        };
        let reference = match self.hud_frame.dimension {
            1 => "progress.nether_loading_progress_screen",
            2 => "progress.theend_loading_progress_screen",
            _ => "progress.overworld_loading_progress_screen",
        };
        let translate = |key: &str| runtime.translation(key);
        let text = |key: &str, fallback: &str| {
            Scalar::Text(
                translate(key).map_or_else(|| fallback.to_owned(), |text| text.to_string()),
            )
        };
        let mut data = DataSource::new();
        data.set_strict(true);
        data.set_global(
            "#title_text",
            text(
                "progressScreen.title.connectingExternal",
                "Connecting to external server",
            ),
        );
        data.set_global(
            "#progress_text",
            match stage {
                LoadingStage::Connecting => {
                    text("progressScreen.message.locating", "Locating server")
                }
                LoadingStage::BuildingTerrain => {
                    text("progressScreen.message.building", "Building terrain")
                }
            },
        );
        data.set_global("#bar_animation_visible", Scalar::Bool(true));
        let context = renderer.context().clone();
        let catalog = Arc::clone(renderer.catalog());
        let px = metrics.scale.get() * FONT_DESIGN_PIXEL_TEXELS as f32;
        let inputs = EngineInputs {
            layouts: &mut self.layouts,
            font: &self.font,
            metrics,
            solid_page: self.solid_texture_page,
            safe_area: self.safe_area,
            content,
            translate: &translate,
            language: runtime.text_generation(),
        };
        let out = EngineOutput {
            nodes,
            next,
            overlay: &[],
        };
        let screen = &mut self.form_presentation.hud.loading;
        let view = ViewState::default();
        let frame = renderer.draw(ScreenArt::default(), inputs, out, |env, root| {
            screen.render_with(reference, &catalog, &context, data, (root, px), env, &view)
        })?;
        Ok(frame.is_some())
    }
}

#[cfg(test)]
impl UiPresentationRuntime {
    /// The loading screen's last laid-out draw nodes, in virtual px.
    pub(crate) fn loading_draw_nodes(&self) -> &[json_ui::DrawNode] {
        self.form_presentation.hud.loading.nodes()
    }
}
