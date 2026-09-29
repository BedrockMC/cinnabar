//! OreUI's twelve-column grid (eight when narrow) with its desktop, tablet and
//! narrow paddings, and the spacer steps.

use super::paint::Canvas;

/// Column spans resolved to logical x ranges.
pub(super) struct Grid {
    left: f32,
    column: f32,
    gutter: f32,
    pub(super) narrow: bool,
}

impl Grid {
    /// The grid across `width`: narrow below 70rem, tablet below 128rem.
    pub(super) fn new(canvas: &Canvas<'_>, width: f32) -> Self {
        let rem = canvas.r(1.0);
        let (padding, gutter) = if width < 70.0 * rem {
            (0.8, 0.4)
        } else if width < 128.0 * rem {
            (1.6, 0.8)
        } else {
            (2.4, 1.6)
        };
        let usable = width.min(128.0 * rem) - padding * 2.0 * rem;
        let columns = if width < 70.0 * rem { 8.0 } else { 12.0 };
        Self {
            left: (width - width.min(128.0 * rem)) * 0.5 + padding * rem,
            column: usable / columns,
            gutter: gutter * rem,
            narrow: width < 70.0 * rem,
        }
    }

    /// The x range of `span` columns starting at `start`, inside the gutters.
    pub(super) fn span(&self, start: usize, span: usize) -> [f32; 2] {
        let left = self.left + self.column * start as f32 + self.gutter * 0.5;
        [left, left + self.column * span as f32 - self.gutter]
    }
}

/// Spacer size `step` (1..=8) in logical pixels.
pub(super) fn space(canvas: &Canvas<'_>, step: usize) -> f32 {
    canvas.r(super::theme::SPACE[step.clamp(1, 8) - 1])
}
