//! Boxed top-right toasts that slide in from the right edge and back out before expiry.

use ui::{TextLayoutRequest, TextStyle, Toast};

use super::{HudLayout, UiPresentationError, UiRuntime};

const TOAST_WIDTH: f32 = 160.0;
const TOAST_HEIGHT: f32 = 32.0;
/// Toasts stacked at once; older ones wait their turn.
const MAX_VISIBLE_TOASTS: usize = 5;
/// Slide duration at each end of a toast's life. Needs independent measurement.
const SLIDE_MILLIS: u64 = 600;
/// Text inset from the box's left edge and the wrap width that keeps text inside it.
const TEXT_INSET: f32 = 8.0;
const TEXT_WRAP: f32 = TOAST_WIDTH - 2.0 * TEXT_INSET;
const TITLE_COLOR: [u8; 4] = [255, 255, 85, 255];
const FILL: [u8; 4] = [24, 24, 24, 235];
const BORDER: [u8; 4] = [92, 92, 92, 255];

/// Horizontal slide-in offset in GUI px: full width when just arrived or about to leave, 0 between.
pub(super) fn slide_offset(toast: &Toast, now_millis: u64) -> f32 {
    let elapsed = now_millis.saturating_sub(toast.received_millis);
    let remaining = toast.expires_millis.saturating_sub(now_millis);
    let progress = (elapsed.min(remaining).min(SLIDE_MILLIS)) as f32 / SLIDE_MILLIS as f32;
    // Ease-out so the box decelerates as it settles.
    let eased = 1.0 - (1.0 - progress) * (1.0 - progress);
    TOAST_WIDTH * (1.0 - eased)
}

impl HudLayout<'_> {
    pub(super) fn toasts(
        &mut self,
        runtime: &UiRuntime,
        now_millis: u64,
    ) -> Result<(), UiPresentationError> {
        let mut top = 0.0;
        let visible = runtime
            .hud()
            .toasts()
            .iter()
            .filter(|toast| toast.visible_at(now_millis))
            .take(MAX_VISIBLE_TOASTS);
        for toast in visible {
            let left = self.geometry.gui_width - TOAST_WIDTH + slide_offset(toast, now_millis);
            let title = self.toast_text(&toast.title)?;
            let message = self.toast_text(&toast.message)?;
            let gui_scale = self.geometry.scale;
            let line = move |layout: &ui::TextLayout| layout.size_64()[1] as f32 / 64.0 / gui_scale;
            let title_height = line(&title);
            let height = TOAST_HEIGHT.max(2.0 * 7.0 + title_height + line(&message));
            self.solid_gui([left, top], [TOAST_WIDTH, height], BORDER)?;
            self.solid_gui(
                [left + 1.0, top + 1.0],
                [TOAST_WIDTH - 2.0, height - 2.0],
                FILL,
            )?;
            self.text_gui_shadowed(title, [left + TEXT_INSET, top + 7.0], TITLE_COLOR)?;
            self.text_gui_shadowed(
                message,
                [left + TEXT_INSET, top + 7.0 + title_height],
                [255; 4],
            )?;
            top += height;
        }
        Ok(())
    }

    fn toast_text(
        &mut self,
        text: &str,
    ) -> Result<std::sync::Arc<ui::TextLayout>, UiPresentationError> {
        let scale = self.text_scale(9.0);
        self.layouts
            .layout(TextLayoutRequest {
                text: super::super::bounded_visible_text(text),
                style: TextStyle::default(),
                width_64: (TEXT_WRAP * self.geometry.scale * 64.0) as u32,
                line_height_64: super::super::TEXT_LINE_HEIGHT_64,
                baseline_64: super::super::TEXT_BASELINE_64,
                scale,
                font: self.font,
            })
            .map_err(UiPresentationError::Text)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;

    fn toast() -> Toast {
        Toast::new(Arc::from("t"), Arc::from("m"), 1, 1_000)
    }

    #[test]
    fn toast_slides_in_rests_then_slides_out() {
        let toast = toast();
        assert_eq!(slide_offset(&toast, 1_000), TOAST_WIDTH);
        assert!(slide_offset(&toast, 1_300) < TOAST_WIDTH);
        assert_eq!(slide_offset(&toast, 3_000), 0.0);
        assert!(slide_offset(&toast, toast.expires_millis - 100) > 0.0);
        assert_eq!(slide_offset(&toast, toast.expires_millis), TOAST_WIDTH);
    }
}
