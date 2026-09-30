//! Per-frame text metrics shared by every HUD, chat, scoreboard and nametag run.

use assets::RuntimeFontCatalog;
use ui::{DpiScale, TextLayoutRequest, TextShadow, TextStyle, UiScale};

use super::gui_scale;

// The compiled Monocraft atlas is rasterized at 18 px/em (see
// `assets/ui-font-source.json`). Monocraft draws on a 60-font-unit grid against
// a 1080-unit em, so one design pixel is two texels: ASCII ink is 16 texels
// tall, 14 of them above the baseline, and the widest advance is 12. That makes
// `UiScale` 1 already equal to Mojang's GUI scale 2, and only whole numbers of
// physical pixels per texel keep every design pixel on a pixel boundary.
pub(super) const FONT_DESIGN_PIXEL_TEXELS: u32 = 2;
pub(super) const FONT_ASCENT_TEXELS: u32 = 14;
pub(super) const FONT_INK_TEXELS: u32 = 16;
/// Mojang pitches chat one design pixel below the font's ink height -- 9 px for
/// an 8 px font. The same ratio against Monocraft's 16 texels gives 18.
pub(super) const TEXT_LINE_HEIGHT_64: u32 = (FONT_INK_TEXELS + FONT_DESIGN_PIXEL_TEXELS) * 64;
/// Distance from the top of a line box down to the baseline, so glyphs sit
/// inside the box instead of hanging above its origin.
pub(super) const TEXT_BASELINE_64: u32 = FONT_ASCENT_TEXELS * 64;
/// Mojang offsets the shadow by exactly one design pixel on both axes.
pub(super) const TEXT_SHADOW_OFFSET_64: u32 = FONT_DESIGN_PIXEL_TEXELS * 64;
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
        }
    }

    pub(super) const fn shadow(&self) -> TextShadow {
        self.shadow
    }
}
