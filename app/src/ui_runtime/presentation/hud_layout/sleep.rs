//! Bed screen: the sleep darkening tint and the centered "Leave Bed" button.

use ui::{SafeArea, TextLayoutRequest, TextStyle, UiRect};

use super::{HudFrame, HudGeometry, HudLayout, UiPresentationError, rect};

/// Fade-in while asleep and fade-out after waking. Needs independent measurement.
const FADE_IN_MILLIS: u64 = 5_000;
const FADE_OUT_MILLIS: u64 = 500;
/// Tint colour and its peak opacity. Needs independent measurement.
const TINT: [u8; 3] = [16, 16, 48];
const PEAK_ALPHA: f32 = 0.7;

const BUTTON_SIZE: [f32; 2] = [200.0, 20.0];
const BUTTON_BOTTOM_MARGIN: f32 = 40.0;
const BUTTON_BORDER: [u8; 4] = [0, 0, 0, 255];
const BUTTON_FILL: [u8; 4] = [111, 111, 111, 255];
const BUTTON_LABEL: &str = "Leave Bed";

/// Tint strength over time, derived from the local sleeping flag.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct SleepTimeline {
    asleep_since: Option<u64>,
    /// Wake time and the strength it started fading from.
    fading: Option<(u64, f32)>,
}

impl SleepTimeline {
    pub(crate) fn observe(&mut self, sleeping: bool, now_millis: u64) {
        match (sleeping, self.asleep_since) {
            (true, None) => {
                self.asleep_since = Some(now_millis);
                self.fading = None;
            }
            (false, Some(_)) => {
                self.fading = Some((now_millis, self.strength(now_millis)));
                self.asleep_since = None;
            }
            _ => {}
        }
    }

    pub(crate) const fn is_sleeping(&self) -> bool {
        self.asleep_since.is_some()
    }

    /// Tint opacity factor in `0.0..=1.0`.
    pub(crate) fn strength(&self, now_millis: u64) -> f32 {
        if let Some(since) = self.asleep_since {
            let elapsed = now_millis.saturating_sub(since);
            return (elapsed as f32 / FADE_IN_MILLIS as f32).min(1.0);
        }
        self.fading.map_or(0.0, |(woke, from)| {
            let elapsed = now_millis.saturating_sub(woke);
            from * (1.0 - (elapsed as f32 / FADE_OUT_MILLIS as f32).min(1.0))
        })
    }
}

fn button_origin(gui_width: f32, gui_height: f32) -> [f32; 2] {
    [
        (gui_width - BUTTON_SIZE[0]) / 2.0,
        gui_height - BUTTON_BOTTOM_MARGIN,
    ]
}

/// Window-logical bounds of the button, for pointer hit testing.
pub(in crate::ui_runtime::presentation) fn leave_bed_bounds(
    geometry: &HudGeometry,
    safe_area: SafeArea,
) -> Option<UiRect> {
    let [x, y] = button_origin(geometry.gui_width, geometry.gui_height);
    let scale = geometry.scale;
    rect(
        safe_area.left() + x * scale,
        safe_area.top() + y * scale,
        safe_area.left() + (x + BUTTON_SIZE[0]) * scale,
        safe_area.top() + (y + BUTTON_SIZE[1]) * scale,
    )
    .ok()
}

impl HudLayout<'_> {
    /// Draws the tint under the HUD and, while asleep, the Leave Bed button.
    pub(super) fn sleep_overlay(&mut self, frame: &HudFrame) -> Result<(), UiPresentationError> {
        let g = self.geometry;
        let strength = frame.sleep.strength(frame.now_millis);
        let alpha = (PEAK_ALPHA * strength * 255.0).round() as u8;
        if alpha > 0 {
            self.solid_gui(
                [0.0, 0.0],
                [g.gui_width, g.gui_height],
                [TINT[0], TINT[1], TINT[2], alpha],
            )?;
        }
        if !frame.sleep.is_sleeping() {
            return Ok(());
        }
        let [x, y] = button_origin(g.gui_width, g.gui_height);
        self.solid_gui([x, y], BUTTON_SIZE, BUTTON_BORDER)?;
        self.solid_gui(
            [x + 1.0, y + 1.0],
            [BUTTON_SIZE[0] - 2.0, BUTTON_SIZE[1] - 2.0],
            BUTTON_FILL,
        )?;
        let scale = self.text_scale(9.0);
        let label = self
            .layouts
            .layout(TextLayoutRequest {
                text: BUTTON_LABEL,
                style: TextStyle::default(),
                width_64: (BUTTON_SIZE[0] * g.scale * 64.0) as u32,
                line_height_64: super::super::TEXT_LINE_HEIGHT_64,
                baseline_64: super::super::TEXT_BASELINE_64,
                scale,
                font: self.font,
            })
            .map_err(UiPresentationError::Text)?;
        let [width, height] = label.size_64().map(|value| value as f32 / 64.0 / g.scale);
        self.text_gui_shadowed(
            label,
            [
                x + (BUTTON_SIZE[0] - width) / 2.0,
                y + (BUTTON_SIZE[1] - height) / 2.0,
            ],
            [255; 4],
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tint_fades_in_while_asleep_and_out_after_waking() {
        let mut timeline = SleepTimeline::default();
        assert_eq!(timeline.strength(0), 0.0);
        timeline.observe(true, 1_000);
        assert_eq!(timeline.strength(1_000), 0.0);
        assert!((timeline.strength(3_500) - 0.5).abs() < 1e-6);
        assert_eq!(timeline.strength(60_000), 1.0);
        timeline.observe(false, 6_000);
        assert!(timeline.strength(6_250) > 0.0 && timeline.strength(6_250) < 1.0);
        assert_eq!(timeline.strength(6_500), 0.0);
        assert!(!timeline.is_sleeping());
    }

    #[test]
    fn waking_early_fades_from_the_reached_strength() {
        let mut timeline = SleepTimeline::default();
        timeline.observe(true, 0);
        timeline.observe(false, 2_500);
        assert!((timeline.strength(2_500) - 0.5).abs() < 1e-6);
        assert!((timeline.strength(2_750) - 0.25).abs() < 1e-6);
    }

    #[test]
    fn button_is_centered_forty_gui_pixels_above_the_bottom() {
        assert_eq!(button_origin(320.0, 240.0), [60.0, 200.0]);
    }
}
