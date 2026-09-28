//! Per-screen interaction state the caller keeps between frames and the layout
//! reads back: which control the pointer hovers or holds, which has focus, and
//! each scroll view's offset. Controls are addressed by their layout key — the
//! `/`-joined instance-name path with a `[index]` suffix on factory instances —
//! so a key survives a re-bind as long as the tree's shape does.

use std::collections::BTreeMap;

/// What the caller tells layout about the live pointer/focus/scroll state.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ViewState {
    pub hovered: Option<String>,
    pub pressed: Option<String>,
    pub focused: Option<String>,
    /// Scroll view key → requested offset in virtual pixels (clamped by layout).
    pub scroll: BTreeMap<String, f64>,
}

impl ViewState {
    pub fn is_hovered(&self, key: &str) -> bool {
        self.hovered.as_deref() == Some(key)
    }

    pub fn is_pressed(&self, key: &str) -> bool {
        self.pressed.as_deref() == Some(key)
    }

    pub fn is_focused(&self, key: &str) -> bool {
        self.focused.as_deref() == Some(key)
    }

    pub fn scroll_offset(&self, key: &str) -> f64 {
        self.scroll.get(key).copied().unwrap_or(0.0)
    }
}

/// A scroll view's measured extents after layout.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ScrollMetrics {
    /// The clamped offset actually applied.
    pub offset: f64,
    pub content: f64,
    pub viewport: f64,
    /// The scrollbar track rect `[x, y, w, h]`, when the template has one.
    pub track: Option<[f64; 4]>,
    /// The drawn scrollbar box rect `[x, y, w, h]`, when it is shown.
    pub thumb: Option<[f64; 4]>,
    /// Pixels scrolled per wheel notch (`scroll_speed`).
    pub speed: f64,
}

impl ScrollMetrics {
    pub fn max_offset(&self) -> f64 {
        (self.content - self.viewport).max(0.0)
    }

    /// The offset that puts the thumb's top at `track_y` (a drag position).
    pub fn offset_for_thumb(&self, thumb_top: f64) -> f64 {
        let (Some(track), Some(thumb)) = (self.track, self.thumb) else {
            return self.offset;
        };
        let travel = (track[3] - thumb[3]).max(0.0);
        if travel <= 0.0 {
            return 0.0;
        }
        ((thumb_top - track[1]) / travel).clamp(0.0, 1.0) * self.max_offset()
    }
}

/// Side results gathered while laying out: every scroll view's metrics.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct LayoutReport {
    pub scrolls: BTreeMap<String, ScrollMetrics>,
}
