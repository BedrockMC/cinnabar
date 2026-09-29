use std::{ops::Range, sync::Arc};

use ui::{UiNode, UiNodeId, UiPoint, UiVisual};

use super::{
    MAX_PRESENTED_CHAT_SUGGESTIONS, TextMetrics, UiPresentationError, UiPresentationRuntime, rect,
};

impl UiPresentationRuntime {
    pub(crate) fn hit_test_chat_suggestion(
        &self,
        position: UiPoint,
        logical_size: [f32; 2],
    ) -> Option<usize> {
        let expected = self.chat_hit_logical_size?;
        if expected.map(f32::to_bits) != logical_size.map(f32::to_bits) {
            return None;
        }
        self.chat_suggestion_hits
            .iter()
            .find_map(|(index, bounds)| bounds.contains(position).then_some(*index))
    }
}

/// One popup row: suggestion index, layout, top, bottom, text colour, selected.
pub(super) type PositionedSuggestion = (usize, Arc<ui::TextLayout>, f32, f32, [u8; 4], bool);

impl UiPresentationRuntime {
    pub(super) fn append_suggestion_nodes(
        &self,
        nodes: &mut Vec<UiNode>,
        next_id: &mut u32,
        positioned_suggestions: &[PositionedSuggestion],
        [chat_left, chat_right]: [f32; 2],
        metrics: TextMetrics,
    ) -> Result<(), UiPresentationError> {
        for (_, layout, y, bottom, color, selected) in positioned_suggestions {
            nodes.push(
                UiNode::new(
                    UiNodeId::new(*next_id),
                    None,
                    rect(chat_left - 2.0, *y, chat_right, *bottom)?,
                )
                .with_visual(UiVisual::Solid {
                    texture_page: self.solid_texture_page,
                    color: if *selected {
                        [96, 96, 96, 224]
                    } else {
                        [0, 0, 0, 192]
                    },
                }),
            );
            *next_id = next_id.saturating_add(1);
            nodes.push(
                UiNode::new(
                    UiNodeId::new(*next_id),
                    None,
                    rect(chat_left, *y, chat_right, *bottom)?,
                )
                .with_visual(UiVisual::Text {
                    layout: Arc::clone(layout),
                    color: *color,
                    shadow: metrics.shadow(),
                }),
            );
            *next_id = next_id.saturating_add(1);
        }
        Ok(())
    }

    pub(crate) fn hit_test_leave_bed(&self, position: UiPoint, logical_size: [f32; 2]) -> bool {
        let expected = self.chat_hit_logical_size;
        expected.is_some_and(|size| size.map(f32::to_bits) == logical_size.map(f32::to_bits))
            && self
                .leave_bed_hit
                .is_some_and(|bounds| bounds.contains(position))
    }
}

pub(super) fn visible_suggestion_range(total: usize, selected: Option<usize>) -> Range<usize> {
    let selected = selected.unwrap_or(0).min(total.saturating_sub(1));
    let end = total.min(
        selected
            .saturating_add(1)
            .max(MAX_PRESENTED_CHAT_SUGGESTIONS),
    );
    end.saturating_sub(MAX_PRESENTED_CHAT_SUGGESTIONS)..end
}
