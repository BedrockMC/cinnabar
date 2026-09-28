//! Centered, magnified, alpha-faded title / subtitle / action bar text.

use ui::{TextLayoutRequest, TextStyle, TimedText, UiScale};

use super::{HudLayout, UiPresentationError, UiRuntime};

/// Title and subtitle magnification over the 9 GUI px text line.
const TITLE_MAGNIFICATION: f32 = 4.0;
const SUBTITLE_MAGNIFICATION: f32 = 2.0;
/// Top of each block relative to the viewport center, in GUI px. Needs independent measurement.
const TITLE_TOP_FROM_CENTER: f32 = -40.0;
const SUBTITLE_TOP_FROM_CENTER: f32 = 10.0;
/// Action bar line top above the viewport bottom, in GUI px; sits on the item label. Needs measurement.
const ACTION_BAR_TOP_FROM_BOTTOM: f32 = 68.0;
/// Below this opacity the text is invisible and is skipped.
const MIN_VISIBLE_ALPHA: u8 = 4;

impl HudLayout<'_> {
    pub(super) fn titles(
        &mut self,
        runtime: &UiRuntime,
        now_millis: u64,
    ) -> Result<(), UiPresentationError> {
        let hud = runtime.hud();
        let center = self.geometry.gui_height / 2.0;
        // A subtitle only presents beneath a live title.
        if let Some(title) = hud.title().filter(|title| title.visible_at(now_millis)) {
            self.magnified_line(
                title,
                now_millis,
                TITLE_MAGNIFICATION,
                center + TITLE_TOP_FROM_CENTER,
            )?;
            if let Some(subtitle) = hud.subtitle().filter(|text| text.visible_at(now_millis)) {
                // The subtitle shares the title's clock so the pair fades together.
                let alpha = title.alpha_at(now_millis);
                self.centered_line(
                    subtitle,
                    SUBTITLE_MAGNIFICATION,
                    center + SUBTITLE_TOP_FROM_CENTER,
                    alpha,
                )?;
            }
        }
        if let Some(bar) = hud.actionbar().filter(|bar| bar.visible_at(now_millis)) {
            let top = self.geometry.gui_height - ACTION_BAR_TOP_FROM_BOTTOM;
            self.magnified_line(bar, now_millis, 1.0, top)?;
        }
        Ok(())
    }

    fn magnified_line(
        &mut self,
        text: &TimedText,
        now_millis: u64,
        magnification: f32,
        top: f32,
    ) -> Result<(), UiPresentationError> {
        self.centered_line(text, magnification, top, text.alpha_at(now_millis))
    }

    fn centered_line(
        &mut self,
        text: &TimedText,
        magnification: f32,
        top: f32,
        alpha: u8,
    ) -> Result<(), UiPresentationError> {
        if alpha < MIN_VISIBLE_ALPHA {
            return Ok(());
        }
        let g = self.geometry;
        let ratio = (9.0 * magnification * g.scale / self.text_line_logical)
            .clamp(UiScale::MIN, UiScale::DISPLAY_MAX);
        let scale = UiScale::new_display(ratio).unwrap_or_default();
        let layout = self
            .layouts
            .layout(TextLayoutRequest {
                text: super::super::bounded_visible_text(&text.text),
                style: TextStyle::default(),
                width_64: (g.gui_width.max(1.0) * g.scale * 64.0) as u32,
                line_height_64: super::super::TEXT_LINE_HEIGHT_64,
                baseline_64: super::super::TEXT_BASELINE_64,
                scale,
                font: self.font,
            })
            .map_err(UiPresentationError::Text)?;
        let width = layout.size_64()[0] as f32 / 64.0 / g.scale;
        self.text_gui_shadowed(
            layout,
            [(g.gui_width - width) / 2.0, top],
            [255, 255, 255, alpha],
        )
    }
}
