//! Per-frame text metrics shared by every HUD, chat, scoreboard and nametag run.

use assets::RuntimeFontCatalog;
use ui::{DpiScale, TextLayoutRequest, TextShadow, TextStyle, UiScale};

use super::gui_scale;

pub(super) use ui::{
    FONT_DESIGN_PIXEL_TEXELS, TEXT_BASELINE_64, TEXT_LINE_HEIGHT_64, TEXT_SHADOW_OFFSET_64,
};

/// Per-frame text metrics shared by every HUD, chat, and scoreboard run so a
/// single frame cannot mix scales or line pitches. Font atlas texels are two
/// texels per Java GUI design pixel, while sprite geometry uses one GUI pixel.
#[derive(Clone, Copy)]
pub(crate) struct TextMetrics {
    pub(super) scale: UiScale,
    pub(super) line_height_64: u32,
    pub(super) baseline_64: u32,
    shadow: TextShadow,
}

impl TextMetrics {
    /// Uses the same GUI-scale choice as sprite geometry. The font atlas
    /// is authored at two texels per GUI design pixel, so its logical scale is
    /// half the sprite scale before the platform DPI is removed.
    pub(super) fn for_viewport(
        physical_size: [u32; 2],
        dpi_scale: DpiScale,
        preference: Option<u8>,
    ) -> Self {
        let dpi = dpi_scale.get();
        let k = gui_scale(physical_size, preference) as f32;
        let scale = (k / (FONT_DESIGN_PIXEL_TEXELS as f32 * dpi)).clamp(UiScale::MIN, UiScale::MAX);
        Self {
            scale: UiScale::new(scale).expect("the clamped scale is inside the UiScale range"),
            line_height_64: TEXT_LINE_HEIGHT_64,
            baseline_64: TEXT_BASELINE_64,
            shadow: TextShadow::Offset64(TEXT_SHADOW_OFFSET_64),
        }
    }

    pub(super) fn request<'a>(
        &self,
        text: &'a str,
        width_64: u32,
        font: &'a RuntimeFontCatalog,
    ) -> TextLayoutRequest<'a> {
        TextLayoutRequest {
            text,
            style: TextStyle::default(),
            width_64,
            line_height_64: self.line_height_64,
            baseline_64: self.baseline_64,
            scale: self.scale,
            font,
            wrap: Default::default(),
        }
    }

    pub(super) const fn shadow(&self) -> TextShadow {
        self.shadow
    }
}
