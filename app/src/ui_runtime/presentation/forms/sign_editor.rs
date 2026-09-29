//! The sign editor screen: a dimmed backdrop, the sign board with its four editable lines and
//! caret, and a Done button. The board look is programmatic; the pack's sign screen layout is
//! not driven yet.

use std::sync::Arc;

use ui::{UiNode, UiRect};

use super::{
    super::{TextMetrics, UiPresentationError, UiPresentationRuntime, rect},
    fallback::{clip, fit_line, solid, text, window_rect},
};
use crate::ui_runtime::{UiRuntime, sign_editor::SIGN_LINES};

/// The board is 96x48 design pixels; each design pixel is two logical pixels at UI scale 1.
const BOARD_DESIGN_WIDTH: f32 = 96.0;
const BOARD_DESIGN_HEIGHT: f32 = 48.0;
const LINE_PITCH_DESIGN: f32 = 10.0;
const FIRST_LINE_DESIGN: f32 = 4.0;
const BOARD_COLOR: [u8; 4] = [161, 127, 80, 255];
const BOARD_EDGE_COLOR: [u8; 4] = [91, 71, 45, 255];
const BACKDROP_COLOR: [u8; 4] = [0, 0, 0, 160];
const BUTTON_COLOR: [u8; 4] = [55, 112, 151, 255];
const LABEL_COLOR: [u8; 4] = [239, 243, 247, 255];

impl UiPresentationRuntime {
    /// Logical window rect of the Done button from the last build; `None` while closed.
    pub(crate) fn sign_editor_done_hit(&self) -> Option<UiRect> {
        self.sign_editor_done
    }

    #[allow(clippy::too_many_arguments)]
    pub(in crate::ui_runtime::presentation) fn append_sign_editor(
        &mut self,
        runtime: &UiRuntime,
        nodes: &mut Vec<UiNode>,
        next: &mut u32,
        metrics: TextMetrics,
        width: f32,
        height: f32,
    ) -> Result<(), UiPresentationError> {
        self.sign_editor_done = None;
        let Some(edit) = runtime.sign_editor().active() else {
            return Ok(());
        };
        if width < 200.0 || height < 200.0 {
            return Ok(());
        }
        let design = (metrics.scale.get() * 2.0)
            .min((width - 32.0) / BOARD_DESIGN_WIDTH)
            .max(1.0);
        let board_width = BOARD_DESIGN_WIDTH * design;
        let board_height = BOARD_DESIGN_HEIGHT * design;
        let left = (width - board_width) * 0.5;
        let top = (height - board_height) * 0.5 - 24.0;
        solid(
            nodes,
            next,
            self.solid_texture_page,
            rect(0.0, 0.0, width, height)?,
            BACKDROP_COLOR,
        );
        let edge = design.max(2.0);
        solid(
            nodes,
            next,
            self.solid_texture_page,
            rect(
                left - edge,
                top - edge,
                left + board_width + edge,
                top + board_height + edge,
            )?,
            BOARD_EDGE_COLOR,
        );
        solid(
            nodes,
            next,
            self.solid_texture_page,
            rect(left, top, left + board_width, top + board_height)?,
            BOARD_COLOR,
        );
        let title = fit_line(self, metrics, "Edit Sign Message", width - 32.0)?;
        let title_top = top - edge - 40.0;
        let title_clip = clip(nodes, next, rect(0.0, title_top, width, title_top + 32.0)?);
        text(
            nodes,
            next,
            title_clip,
            title,
            metrics,
            rect(16.0, 0.0, width - 16.0, 32.0)?,
            LABEL_COLOR,
        );
        let (cursor_line, cursor_column) = edit.cursor();
        let color = edit.color();
        let wrap = (board_width * 64.0 * 4.0) as u32;
        let board_clip = clip(
            nodes,
            next,
            rect(left, top, left + board_width, top + board_height)?,
        );
        for index in 0..SIGN_LINES {
            let line = &edit.lines()[index];
            let layout = self
                .layouts
                .layout(metrics.request(line, wrap, &self.font))
                .map_err(UiPresentationError::Text)?;
            let line_width = layout.size_64()[0] as f32 / 64.0;
            let line_height = layout.size_64()[1] as f32 / 64.0;
            let x = (board_width - line_width) * 0.5;
            let y = (FIRST_LINE_DESIGN + index as f32 * LINE_PITCH_DESIGN) * design;
            if !line.is_empty() {
                text(
                    nodes,
                    next,
                    board_clip,
                    Arc::clone(&layout),
                    metrics,
                    rect(x, y, x + line_width + 2.0, y + line_height)?,
                    color,
                );
            }
            if index == cursor_line {
                let prefix: String = line.chars().take(cursor_column).collect();
                let prefix_width = self
                    .layouts
                    .layout(metrics.request(&prefix, wrap, &self.font))
                    .map_err(UiPresentationError::Text)?
                    .size_64()[0] as f32
                    / 64.0;
                let caret_x = x + prefix_width;
                let caret = rect(
                    caret_x,
                    y,
                    caret_x + (design * 0.5).max(1.0),
                    y + line_height.max(design * 8.0),
                )?;
                solid(
                    nodes,
                    next,
                    self.solid_texture_page,
                    offset(caret, left, top)?,
                    color,
                );
            }
        }
        let button_width = board_width.min(200.0 * (design / 2.0).max(1.0));
        let button_height = (design * 10.0).max(28.0);
        let button_left = (width - button_width) * 0.5;
        let button_top = top + board_height + edge + 24.0;
        let button = rect(
            button_left,
            button_top,
            button_left + button_width,
            button_top + button_height,
        )?;
        solid(nodes, next, self.solid_texture_page, button, BUTTON_COLOR);
        let label = fit_line(self, metrics, "Done", button_width - 24.0)?;
        let label_clip = clip(nodes, next, button);
        text(
            nodes,
            next,
            label_clip,
            label,
            metrics,
            rect(
                button_left + 12.0,
                button_top + 6.0,
                button_left + button_width - 12.0,
                button_top + button_height,
            )?,
            LABEL_COLOR,
        );
        self.sign_editor_done = Some(window_rect(button, self.safe_area)?);
        Ok(())
    }
}

/// `bounds` moved right and down by `dx`, `dy`.
fn offset(bounds: UiRect, dx: f32, dy: f32) -> Result<UiRect, UiPresentationError> {
    rect(
        bounds.min().x() + dx,
        bounds.min().y() + dy,
        bounds.max().x() + dx,
        bounds.max().y() + dy,
    )
}
