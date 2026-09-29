//! The OreUI canvas: fills, one-texel edges, speculars and bevels, scaled text,
//! and (in the local-originals mode) sprites from the install's atlases.

use std::collections::HashMap;

use ui::{TextLayoutCache, TextShadow, UiNode, UiNodeId, UiRect, UiScale, UiVisual};

use super::super::super::{
    FONT_DESIGN_PIXEL_TEXELS, IconRef, TextMetrics, UiPresentationError, rect,
};
use super::theme::{EDGE, Rgba, TEXT_SHADOW, Type};
use crate::menu::MenuAction;

/// A logical-pixel rect `[left, top, right, bottom]`.
pub(super) type Bounds = [f32; 4];

/// The install's atlas sprites on one texture page (local-originals mode).
pub(crate) struct Originals {
    pub(super) page: u16,
    pub(super) sprites: HashMap<String, [u16; 4]>,
}

pub(super) struct Canvas<'a> {
    pub(super) nodes: &'a mut Vec<UiNode>,
    pub(super) next: &'a mut u32,
    pub(super) layouts: &'a mut TextLayoutCache,
    pub(super) font: &'a assets::RuntimeFontCatalog,
    pub(super) metrics: TextMetrics,
    pub(super) solid_page: u16,
    pub(super) originals: Option<&'a Originals>,
    /// Logical pixels per rem (five GUI pixels).
    pub(super) rem: f32,
    pub(super) hits: Vec<(MenuAction, UiRect)>,
}

impl<'a> Canvas<'a> {
    pub(super) fn new(
        nodes: &'a mut Vec<UiNode>,
        next: &'a mut u32,
        layouts: &'a mut TextLayoutCache,
        font: &'a assets::RuntimeFontCatalog,
        metrics: TextMetrics,
        solid_page: u16,
        originals: Option<&'a Originals>,
    ) -> Self {
        let gui_pixel = metrics.scale.get() * FONT_DESIGN_PIXEL_TEXELS as f32;
        Self {
            nodes,
            next,
            layouts,
            font,
            metrics,
            solid_page,
            originals,
            rem: gui_pixel * 5.0,
            hits: Vec::new(),
        }
    }

    /// Logical pixels for `value` rem.
    pub(super) fn r(&self, value: f32) -> f32 {
        value * self.rem
    }

    fn push(&mut self, bounds: Bounds, visual: UiVisual) -> Result<UiRect, UiPresentationError> {
        let area = rect(bounds[0], bounds[1], bounds[2], bounds[3])?;
        self.nodes
            .push(UiNode::new(UiNodeId::new(*self.next), None, area).with_visual(visual));
        *self.next = self.next.saturating_add(1);
        Ok(area)
    }

    pub(super) fn fill(&mut self, bounds: Bounds, color: Rgba) -> Result<(), UiPresentationError> {
        if bounds[2] <= bounds[0] || bounds[3] <= bounds[1] || color[3] == 0 {
            return Ok(());
        }
        let texture_page = self.solid_page;
        self.push(
            bounds,
            UiVisual::Solid {
                texture_page,
                color,
            },
        )?;
        Ok(())
    }

    /// A `width`-rem border drawn inside `bounds`.
    pub(super) fn frame(
        &mut self,
        b: Bounds,
        width: f32,
        color: Rgba,
    ) -> Result<(), UiPresentationError> {
        let w = self.r(width);
        self.fill([b[0], b[1], b[2], b[1] + w], color)?;
        self.fill([b[0], b[3] - w, b[2], b[3]], color)?;
        self.fill([b[0], b[1] + w, b[0] + w, b[3] - w], color)?;
        self.fill([b[2] - w, b[1] + w, b[2], b[3] - w], color)
    }

    /// One-texel inner edges: top and left in `top`, bottom and right in `bottom`.
    pub(super) fn specular(
        &mut self,
        b: Bounds,
        top: Rgba,
        bottom: Rgba,
    ) -> Result<(), UiPresentationError> {
        let w = self.r(EDGE);
        self.fill([b[0], b[1], b[2], b[1] + w], top)?;
        self.fill([b[0], b[1] + w, b[0] + w, b[3] - w], top)?;
        self.fill([b[0], b[3] - w, b[2], b[3]], bottom)?;
        self.fill([b[2] - w, b[1] + w, b[2], b[3] - w], bottom)
    }

    /// One-texel top and bottom edges only.
    pub(super) fn bevel(
        &mut self,
        b: Bounds,
        top: Rgba,
        bottom: Rgba,
    ) -> Result<(), UiPresentationError> {
        let w = self.r(EDGE);
        self.fill([b[0], b[1], b[2], b[1] + w], top)?;
        self.fill([b[0], b[3] - w, b[2], b[3]], bottom)
    }

    /// Draws `value` from `at` within `width`; returns the laid-out height.
    pub(super) fn text(
        &mut self,
        value: &str,
        at: [f32; 2],
        width: f32,
        style: Type,
        color: Rgba,
        shadow: bool,
    ) -> Result<f32, UiPresentationError> {
        if value.is_empty() {
            return Ok(0.0);
        }
        let mut request = self
            .metrics
            .request(value, (width.max(1.0) * 64.0) as u32, self.font);
        // The open font's default line is the 1.6rem body size.
        if let Ok(scale) = UiScale::new(self.metrics.scale.get() * style.size / 1.6) {
            request.scale = scale;
        }
        let layout = self
            .layouts
            .layout(request)
            .map_err(UiPresentationError::Text)?;
        let height = layout.size_64()[1] as f32 / 64.0;
        if shadow {
            let offset = self.r(EDGE);
            self.push(
                [
                    at[0] + offset,
                    at[1] + offset,
                    at[0] + offset + width.max(1.0),
                    at[1] + offset + height.max(1.0),
                ],
                UiVisual::Text {
                    layout: layout.clone(),
                    color: TEXT_SHADOW,
                    shadow: TextShadow::None,
                },
            )?;
        }
        self.push(
            [
                at[0],
                at[1],
                at[0] + width.max(1.0),
                at[1] + height.max(1.0),
            ],
            UiVisual::Text {
                layout,
                color,
                shadow: TextShadow::None,
            },
        )?;
        Ok(height)
    }

    /// The width `value` lays out to in `style`.
    pub(super) fn measure(&mut self, value: &str, style: Type) -> Result<f32, UiPresentationError> {
        let mut request = self.metrics.request(value, 65_536 * 64, self.font);
        if let Ok(scale) = UiScale::new(self.metrics.scale.get() * style.size / 1.6) {
            request.scale = scale;
        }
        let layout = self
            .layouts
            .layout(request)
            .map_err(UiPresentationError::Text)?;
        Ok(layout.size_64()[0] as f32 / 64.0)
    }

    /// `value` centred in `bounds` on one line.
    pub(super) fn text_centred(
        &mut self,
        value: &str,
        b: Bounds,
        style: Type,
        color: Rgba,
        shadow: bool,
    ) -> Result<(), UiPresentationError> {
        let width = self.measure(value, style)?.min(b[2] - b[0]);
        let height = self.r(style.line);
        let at = [
            (b[0] + b[2] - width) * 0.5,
            (b[1] + b[3] - height) * 0.5 + self.r((style.line - style.size) * 0.5),
        ];
        self.text(value, at, width + 1.0, style, color, shadow)?;
        Ok(())
    }

    /// Draws an install sprite (local-originals mode); `false` when unavailable.
    pub(super) fn sprite(
        &mut self,
        key: &str,
        b: Bounds,
        color: Rgba,
    ) -> Result<bool, UiPresentationError> {
        let Some(originals) = self.originals else {
            return Ok(false);
        };
        let Some(&uv) = originals.sprites.get(key) else {
            return Ok(false);
        };
        let texture_page = originals.page;
        self.push(
            b,
            UiVisual::Sprite {
                texture_page,
                uv,
                color,
            },
        )?;
        Ok(true)
    }

    /// A caller-supplied image (artwork, gamerpic) stretched over `b`.
    pub(super) fn icon_ref(&mut self, icon: IconRef, b: Bounds) -> Result<(), UiPresentationError> {
        self.push(
            b,
            UiVisual::Sprite {
                texture_page: icon.page,
                uv: icon.uv,
                color: [255; 4],
            },
        )?;
        Ok(())
    }

    pub(super) fn hit(&mut self, action: MenuAction, b: Bounds) -> Result<(), UiPresentationError> {
        let area = rect(b[0], b[1], b[2], b[3])?;
        self.hits.push((action, area));
        Ok(())
    }
}
