//! F3 debug overlay: a left and a right text column over translucent row strips.

use ui::{TextShadow, UiNode, UiNodeId, UiVisual};

use super::{TextMetrics, UiPresentationError, UiPresentationRuntime, bounded_visible_text, rect};

/// Row strip behind each line. Needs independent measurement.
const STRIP_COLOR: [u8; 4] = [80, 80, 80, 144];
/// Text lines sit two GUI px in from the corner; a line is nine GUI px tall.
const INSET_GUI_PX: f32 = 2.0;
const LINE_GUI_PX: f32 = 9.0;
const MAX_LINES_PER_COLUMN: usize = 40;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct DebugLines {
    pub left: Vec<String>,
    pub right: Vec<String>,
}

impl UiPresentationRuntime {
    pub(crate) fn set_debug_lines(&mut self, lines: Option<DebugLines>) {
        self.debug_lines = lines;
    }

    pub(super) fn append_debug_overlay(
        &mut self,
        nodes: &mut Vec<UiNode>,
        next_id: &mut u32,
        metrics: TextMetrics,
        content_width: f32,
    ) -> Result<(), UiPresentationError> {
        let Some(lines) = self.debug_lines.as_ref() else {
            return Ok(());
        };
        let width_64 = (content_width.max(1.0) * 64.0) as u32;
        for (column, right_aligned) in [(&lines.left, false), (&lines.right, true)] {
            for (index, line) in column.iter().take(MAX_LINES_PER_COLUMN).enumerate() {
                let layout = self
                    .layouts
                    .layout(metrics.request(bounded_visible_text(line), width_64, &self.font))
                    .map_err(UiPresentationError::Text)?;
                let [text_width, height] = layout.size_64().map(|value| value as f32 / 64.0);
                let pixel = height / LINE_GUI_PX;
                let left = if right_aligned {
                    (content_width - INSET_GUI_PX * pixel - text_width).max(0.0)
                } else {
                    INSET_GUI_PX * pixel
                };
                let top = INSET_GUI_PX * pixel + height * index as f32;
                let strip = UiNode::new(
                    UiNodeId::new(*next_id),
                    None,
                    rect(left - pixel, top, left + text_width + pixel, top + height)?,
                )
                .with_visual(UiVisual::Solid {
                    texture_page: self.solid_texture_page,
                    color: STRIP_COLOR,
                });
                let text = UiNode::new(
                    UiNodeId::new(next_id.saturating_add(1)),
                    None,
                    rect(left, top, left + text_width, top + height)?,
                )
                .with_visual(UiVisual::Text {
                    layout,
                    color: [255; 4],
                    shadow: TextShadow::None,
                });
                nodes.push(strip);
                nodes.push(text);
                *next_id = next_id.saturating_add(2);
            }
        }
        Ok(())
    }
}
