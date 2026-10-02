//! Label and hover-text painting after vanilla's `TextComponent` and
//! `HoverTextRenderer`: one layout per label with per-line alignment, line
//! padding, hyphen chops and `...` at the lines its height holds.

use std::{borrow::Cow, cell::RefCell, sync::Arc};

use assets::RuntimeFontCatalog;
use json_ui::{LabelShape, TextAlign, TextMeasure, TextOptions, TextureSource};
use ui::{
    TextLayoutCache, TextLayoutRequest, TextLineAlign, TextShadow, TextWrap, UiNode, UiScale,
    UiVisual, WordChop,
};

use super::super::super::{TextMetrics, UiPresentationError, rect};
use super::Painter;

/// Largest wrap width handed to the text layout (logical px), for "no wrap".
pub(super) const UNWRAPPED_LOGICAL: f64 = 65_536.0;
/// Hover text: pointer offset, box padding and text inset, GUI px (1.26.50 `HoverTextRenderer`).
const TOOLTIP_OFFSET: [f32; 2] = [10.0, -10.0];
const TOOLTIP_PAD: [f32; 2] = [9.0, 8.0];
const TOOLTIP_INSET: f32 = 5.0;
const TOOLTIP_TEXTURE: &str = "textures/ui/purpleBorder";
const TOOLTIP_FALLBACK: [u8; 4] = [16, 0, 16, 224];

#[derive(Clone)]
pub(super) struct TextPaint {
    pub(super) color: [u8; 4],
    pub(super) shadow: TextShadow,
    pub(super) align: TextAlign,
    pub(super) scale: f32,
    pub(super) localize: bool,
    pub(super) options: TextOptions,
}

/// A label's text after vanilla localization; empty lines drop as the vanilla label drops them.
pub(super) fn localized<'a>(
    text: &'a str,
    translate: &dyn Fn(&str) -> Option<Arc<str>>,
) -> Cow<'a, str> {
    let text = json_ui::localize_text(text, translate);
    if text.contains("\n\n") || text.starts_with('\n') || text.ends_with('\n') {
        let lines: Vec<&str> = text.split('\n').filter(|line| !line.is_empty()).collect();
        return Cow::Owned(lines.join("\n"));
    }
    text
}

/// `request` at `factor` times the metrics' scale; a factor the display range
/// cannot hold keeps the base scale.
pub(super) fn scaled_request<'a>(
    metrics: &TextMetrics,
    text: &'a str,
    width_64: u32,
    font: &'a RuntimeFontCatalog,
    factor: f32,
) -> TextLayoutRequest<'a> {
    let mut request = metrics.request(text, width_64, font);
    if factor != 1.0
        && let Ok(scale) = UiScale::new_display(metrics.scale.get() * factor)
    {
        request.scale = scale;
    }
    request
}

/// A vanilla label's request: hyphen chops, `line_padding` in logical px.
fn label_request<'a>(
    metrics: &TextMetrics,
    text: &'a str,
    width: f64,
    font: &'a RuntimeFontCatalog,
    shape: LabelShape,
    px: f32,
) -> TextLayoutRequest<'a> {
    let mut request = scaled_request(metrics, text, width_64(width), font, shape.scale as f32);
    request.wrap = TextWrap {
        line_padding_64: (shape.line_padding * f64::from(px) * 64.0).round() as i32,
        chop: if shape.hide_hyphen {
            WordChop::Bare
        } else {
            WordChop::Hyphen
        },
        ..TextWrap::default()
    };
    request
}

/// Rounded up, so text laid out at its own measured width does not wrap.
pub(super) fn width_64(logical: f64) -> u32 {
    (logical.clamp(1.0, UNWRAPPED_LOGICAL) * 64.0).ceil() as u32
}

pub(super) struct Measure<'a, 'b> {
    pub(super) layouts: &'b RefCell<&'a mut TextLayoutCache>,
    pub(super) font: &'a RuntimeFontCatalog,
    pub(super) metrics: TextMetrics,
    pub(super) px: f32,
    pub(super) translate: &'a dyn Fn(&str) -> Option<Arc<str>>,
}

impl TextMeasure for Measure<'_, '_> {
    fn extent(&self, text: &str) -> [f64; 2] {
        self.wrapped(text, UNWRAPPED_LOGICAL / f64::from(self.px))
    }

    fn wrapped(&self, text: &str, max_width: f64) -> [f64; 2] {
        self.label(
            text,
            Some(max_width),
            LabelShape {
                scale: 1.0,
                line_padding: 0.0,
                hide_hyphen: false,
            },
        )
    }

    fn label(&self, text: &str, max_width: Option<f64>, shape: LabelShape) -> [f64; 2] {
        if text.is_empty() {
            return [0.0, 0.0];
        }
        let px = f64::from(self.px);
        let width = max_width
            .filter(|width| *width > 0.0)
            .map_or(UNWRAPPED_LOGICAL, |width| width * px);
        let request = label_request(&self.metrics, text, width, self.font, shape, self.px);
        match self.layouts.borrow_mut().layout(request) {
            Ok(layout) => layout.size_64().map(|size| f64::from(size) / 64.0 / px),
            Err(_) => [0.0, 0.0],
        }
    }

    /// Keeps label shaping while selecting the pack's named font.
    fn named_label(
        &self,
        text: &str,
        font: &str,
        width: Option<f64>,
        shape: LabelShape,
    ) -> [f64; 2] {
        let measure = Measure {
            font: self.font.font_named(font),
            ..*self
        };
        measure.label(text, width, shape)
    }

    fn localize<'t>(&self, text: &'t str) -> Cow<'t, str> {
        localized(text, self.translate)
    }
}

impl Painter<'_> {
    /// A label's text as one layout: lines past its height drop and the last
    /// kept one ends in `...`; each line aligns within the label's width.
    pub(super) fn text(
        &mut self,
        text: &str,
        dest: [f32; 4],
        clip: [f32; 4],
        style: TextPaint,
    ) -> Result<(), UiPresentationError> {
        let text = if style.localize {
            localized(text, self.translate)
        } else {
            Cow::Borrowed(text)
        };
        if text.is_empty() {
            return Ok(());
        }
        let shape = LabelShape {
            scale: f64::from(style.scale),
            line_padding: f64::from(style.options.line_padding),
            hide_hyphen: style.options.hide_hyphen,
        };
        let mut request = label_request(
            &self.metrics,
            &text,
            f64::from(dest[2] - dest[0]),
            self.font
                .font_named(style.options.font_type.as_deref().unwrap_or("default")),
            shape,
            self.px,
        );
        let pitch = (request.line_height_64 as f32 * request.scale.get()
            + request.wrap.line_padding_64 as f32)
            / 64.0;
        let room = ((dest[3] - dest[1]) / pitch.max(1e-3) + 0.01)
            .floor()
            .max(1.0);
        request.wrap.max_lines = Some(room.min(f32::from(u16::MAX)) as u16);
        request.wrap.align = match style.align {
            TextAlign::Left => TextLineAlign::Left,
            TextAlign::Center => TextLineAlign::Center,
            TextAlign::Right => TextLineAlign::Right,
        };
        let Ok(layout) = self.layouts.layout(request) else {
            return Ok(());
        };
        let [width, height] = layout.size_64().map(|size| size as f32 / 64.0);
        let parent = self.group(clip)?;
        let id = self.id();
        self.nodes.push(
            UiNode::new(
                id,
                Some(parent),
                rect(
                    dest[0] - clip[0],
                    dest[1] - clip[1],
                    dest[0] + width.max(1.0) - clip[0],
                    dest[1] + height - clip[1],
                )?,
            )
            .with_visual(UiVisual::Text {
                layout,
                color: style.color,
                shadow: style.shadow,
            }),
        );
        Ok(())
    }

    /// Hover text in a `purpleBorder` box beside the pointer (or the hovered
    /// control), wrapped at `max_width` GUI px when positive, flipped left of
    /// the pointer past the right edge and centred above it past the left.
    pub(super) fn tooltip(
        &mut self,
        text: &str,
        dest: [f32; 4],
        max_width: Option<f64>,
        opacity: f32,
    ) -> Result<(), UiPresentationError> {
        let px = self.px;
        let anchor = self
            .art
            .pointer
            .map_or([dest[2], dest[1]], |point| [point[0] * px, point[1] * px]);
        let wrap = max_width
            .filter(|width| *width >= 1.0)
            .map_or(UNWRAPPED_LOGICAL, |width| width * f64::from(px));
        let request = self.metrics.request(text, width_64(wrap), self.font);
        let Ok(layout) = self.layouts.layout(request) else {
            return Ok(());
        };
        let [w, h] = layout.size_64().map(|size| size as f32 / 64.0);
        let size = [w + TOOLTIP_PAD[0] * px, h + TOOLTIP_PAD[1] * px];
        let mut offset = [TOOLTIP_OFFSET[0] * px, TOOLTIP_OFFSET[1] * px];
        let overflow = anchor[1] + size[1] + offset[1] - self.screen[3];
        if overflow > 0.0 {
            offset[1] -= overflow;
        }
        if anchor[0] + size[0] + offset[0] > self.screen[2] {
            offset[0] = -(offset[0] + size[0]);
        }
        if anchor[0] + offset[0] < 0.0 {
            offset = [-0.5 * size[0], -size[1]];
        }
        let origin = [anchor[0] + offset[0], anchor[1] + offset[1]];
        let alpha = (255.0 * opacity.clamp(0.0, 1.0)).round() as u8;
        self.tooltip_box(
            [
                origin[0],
                origin[1],
                origin[0] + size[0],
                origin[1] + size[1],
            ],
            alpha,
        )?;
        let inset = TOOLTIP_INSET * px;
        let at = [origin[0] + inset, origin[1] + inset];
        self.push(
            UiVisual::Text {
                layout,
                color: [255, 255, 255, alpha],
                shadow: self.metrics.shadow(),
            },
            [at[0], at[1], at[0] + w, at[1] + h],
        )
    }

    /// The tooltip background: the pack's `purpleBorder` nine-slice.
    fn tooltip_box(&mut self, bounds: [f32; 4], alpha: u8) -> Result<(), UiPresentationError> {
        let px = self.px;
        let Some(meta) = self.textures.texture(TOOLTIP_TEXTURE) else {
            let mut color = TOOLTIP_FALLBACK;
            color[3] = (u16::from(color[3]) * u16::from(alpha) / 255) as u8;
            return self.solid(bounds, color);
        };
        let virtual_rect = json_ui::Rect::new(
            f64::from(bounds[0] / px),
            f64::from(bounds[1] / px),
            f64::from((bounds[2] - bounds[0]) / px),
            f64::from((bounds[3] - bounds[1]) / px),
        );
        for quad in json_ui::nine_slice(virtual_rect, &meta) {
            let Some(visual) = self.sprite(
                TOOLTIP_TEXTURE,
                quad.uv,
                [255, 255, 255, alpha],
                json_ui::SpriteFilter::default(),
            ) else {
                continue;
            };
            let dest = self.logical(&quad.dest);
            self.push(visual, dest)?;
        }
        Ok(())
    }
}

/// The format codes in force at the end of `text`, to open the next line with.
pub(in super::super) fn active_codes(text: &str) -> String {
    let mut codes = String::new();
    let mut characters = text.chars();
    while let Some(character) = characters.next() {
        if character != '§' {
            continue;
        }
        match characters.next() {
            Some('r') => codes.clear(),
            Some(code @ ('0'..='9' | 'a'..='w')) => {
                codes.push('§');
                codes.push(code);
            }
            _ => {}
        }
    }
    codes
}
