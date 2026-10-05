//! The own last-forwarded position marker, painted by the private JSON-UI catalog.

mod projection;
pub use projection::project_entity_rectangle;

use json_ui::{Catalog, Context, DataSource, Scalar, ViewState};
use serde_json::json;
use std::sync::Arc;
use ui::{DpiScale, UiNode, UiScale};

use super::super::{FONT_DESIGN_PIXEL_TEXELS, TextMetrics, UiPresentationRuntime};
use super::{
    engine::{EngineInputs, EngineOutput, ScreenArt},
    hud::CachedScreen,
};
use crate::ui_runtime::UiRuntime;

const SCREEN: &str = "cinnabar_position.marker";
const TEMPLATE: &[u8] = br##"{
  "namespace":"cinnabar_position",
  "edge":{
    "type":"image","size":[0,0],"offset":[0,0],
    "anchor_from":"top_left","anchor_to":"top_left",
    "color":[0.9490196078431372,0.47843137254901963,0.38823529411764707,1.0]
  },
  "marker":{
    "type":"panel","size":["100%","100%"],"controls":[
      {"top@cinnabar_position.edge":{"bindings":[{"binding_name":"#top_width","binding_name_override":"#size_binding_x_absolute"},{"binding_name":"#top_height","binding_name_override":"#size_binding_y_absolute"},{"binding_name":"#top_offset","binding_name_override":"#offset"}]}},
      {"bottom@cinnabar_position.edge":{"bindings":[{"binding_name":"#bottom_width","binding_name_override":"#size_binding_x_absolute"},{"binding_name":"#bottom_height","binding_name_override":"#size_binding_y_absolute"},{"binding_name":"#bottom_offset","binding_name_override":"#offset"}]}},
      {"left@cinnabar_position.edge":{"bindings":[{"binding_name":"#left_width","binding_name_override":"#size_binding_x_absolute"},{"binding_name":"#left_height","binding_name_override":"#size_binding_y_absolute"},{"binding_name":"#left_offset","binding_name_override":"#offset"}]}},
      {"right@cinnabar_position.edge":{"bindings":[{"binding_name":"#right_width","binding_name_override":"#size_binding_x_absolute"},{"binding_name":"#right_height","binding_name_override":"#size_binding_y_absolute"},{"binding_name":"#right_offset","binding_name_override":"#offset"}]}}
    ]
  }
}"##;

pub(super) struct ModGhost {
    bounds: Option<[u32; 4]>,
    catalog: Arc<Catalog>,
    screen: CachedScreen,
    data: Arc<DataSource>,
    last_geometry: Option<([u32; 4], f32, [f32; 2])>,
}

impl UiPresentationRuntime {
    /// Sets the own-position rectangle in physical pixels; `None` clears it immediately.
    pub fn set_mod_position_rectangle(&mut self, bounds: Option<[u32; 4]>) -> Result<(), String> {
        let bounds = bounds.filter(|b| b[0] < b[2] && b[1] < b[3]);
        if self.form_presentation.mod_ghost.is_none() && bounds.is_some() {
            let catalog = Catalog::from_files([
                ("ui/_global_variables.json", &b"{}"[..]),
                (
                    "ui/_ui_defs.json",
                    &br#"{"ui_defs":["ui/cinnabar_position.json"]}"#[..],
                ),
                ("ui/cinnabar_position.json", TEMPLATE),
            ])
            .map_err(|error| error.to_string())?;
            self.form_presentation.mod_ghost = Some(ModGhost {
                bounds: None,
                catalog: Arc::new(catalog),
                screen: CachedScreen::default(),
                data: Arc::default(),
                last_geometry: None,
            });
        }
        if let Some(ghost) = self.form_presentation.mod_ghost.as_mut() {
            ghost.bounds = bounds;
        }
        Ok(())
    }

    pub(in super::super) fn append_mod_ghost(
        &mut self,
        player: &player_state::PlayerState,
        runtime: &UiRuntime,
        nodes: &mut Vec<UiNode>,
        next: &mut u32,
        mut metrics: TextMetrics,
        content: [f32; 2],
        dpi_scale: DpiScale,
    ) {
        let Some(ghost) = self.form_presentation.mod_ghost.as_mut() else {
            return;
        };
        let Some(bounds) = ghost.bounds else { return };
        if self.loading_stage.is_some() || runtime.ui_focused(player) {
            return;
        }
        let Some(renderer) = self.form_presentation.engine.as_deref() else {
            return;
        };
        metrics.scale = UiScale::new_display(1.0).expect("unit display scale is valid");
        let px = metrics.scale.get() * FONT_DESIGN_PIXEL_TEXELS as f32;
        let scale = dpi_scale.get() * px;
        let inset = [self.safe_area.left(), self.safe_area.top()];
        let geometry = (bounds, scale, inset);
        if ghost.last_geometry != Some(geometry) {
            ghost.data = Arc::new(rectangle_data(bounds, scale, inset));
            ghost.last_geometry = Some(geometry);
        }
        let data = Arc::clone(&ghost.data);
        let inputs = EngineInputs {
            layouts: &mut self.layouts,
            font: &self.font,
            metrics,
            solid_page: self.solid_texture_page,
            safe_area: self.safe_area,
            content,
            translate: &|_| None,
            language: runtime.text_generation(),
        };
        let rollback = (nodes.len(), *next);
        let out = EngineOutput {
            nodes: &mut *nodes,
            next: &mut *next,
            overlay: &[],
        };
        if let Err(error) = renderer.draw(ScreenArt::default(), inputs, out, |env, root| {
            ghost.screen.render_shared_with(
                SCREEN,
                &ghost.catalog,
                &Context::default(),
                data,
                (root, px, runtime.text_generation()),
                env,
                &ViewState::default(),
            )
        }) {
            nodes.truncate(rollback.0);
            *next = rollback.1;
            ghost.bounds = None;
            bevy::log::warn!(%error, "own-position marker rejected");
        }
    }
}

fn rectangle_data([left, top, right, bottom]: [u32; 4], scale: f32, inset: [f32; 2]) -> DataSource {
    let mut data = DataSource::new();
    let (left, top, right, bottom) = (left as f32, top as f32, right as f32, bottom as f32);
    let edges = [
        ("top", [left, top], [right - left, 1.0]),
        ("bottom", [left, bottom - 1.0], [right - left, 1.0]),
        (
            "left",
            [left, top + 1.0],
            [1.0, (bottom - top - 2.0).max(0.0)],
        ),
        (
            "right",
            [right - 1.0, top + 1.0],
            [1.0, (bottom - top - 2.0).max(0.0)],
        ),
    ];
    for (name, offset, size) in edges {
        data.set_global(
            format!("#{name}_width"),
            Scalar::Num(f64::from(size[0] / scale)),
        );
        data.set_global(
            format!("#{name}_height"),
            Scalar::Num(f64::from(size[1] / scale)),
        );
        data.set_global(
            format!("#{name}_offset"),
            Scalar::Json(json!([
                offset[0] / scale - inset[0] / FONT_DESIGN_PIXEL_TEXELS as f32,
                offset[1] / scale - inset[1] / FONT_DESIGN_PIXEL_TEXELS as f32,
            ])),
        );
    }
    data
}

#[cfg(test)]
mod tests;
