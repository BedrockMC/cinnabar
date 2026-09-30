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

    /// `key`'s interaction bits as [`StateGate::shown`] indexes them.
    pub fn interaction(&self, key: &str) -> u8 {
        u8::from(self.is_hovered(key))
            | u8::from(self.is_pressed(key)) << 1
            | u8::from(self.is_focused(key)) << 2
    }

    /// Whether `other` lays out identically: hover, press and focus only pick
    /// which state children show.
    pub fn same_geometry(&self, other: &ViewState) -> bool {
        self.scroll == other.scroll
    }
}

/// A subtree shown only under some interaction states of its stateful control,
/// so a laid-out screen repaints hover and press without laying out again.
#[derive(Clone, Debug, PartialEq)]
pub struct StateGate {
    /// The stateful control's layout key.
    pub key: String,
    /// The enclosing gate, which must pass too.
    pub parent: Option<u32>,
    /// Bit `ViewState::interaction` set for each state the subtree shows in.
    pub shown: u8,
}

/// Appends a gate on `key`'s interaction inside `parent`, returning its index.
pub(crate) fn push_gate(
    gates: &mut Vec<StateGate>,
    key: &str,
    parent: Option<u32>,
    shown: u8,
) -> Option<u32> {
    let index = u32::try_from(gates.len()).ok();
    gates.push(StateGate {
        key: key.to_owned(),
        parent,
        shown,
    });
    index
}

/// Whether `gate` and every enclosing gate pass under `state`.
pub fn gate_open(gates: &[StateGate], mut gate: Option<u32>, state: &ViewState) -> bool {
    while let Some(entry) = gate.and_then(|index| gates.get(index as usize)) {
        if entry.shown >> state.interaction(&entry.key) & 1 == 0 {
            return false;
        }
        gate = entry.parent;
    }
    true
}

/// A scroll view's measured extents after layout.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ScrollMetrics {
    /// The clamped offset actually applied.
    pub offset: f64,
    pub content: f64,
    pub viewport: f64,
    /// The viewport's top edge (virtual px), for scrolling a control into view.
    pub viewport_top: f64,
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

    /// The offset that brings the span `[top, bottom)` (current coordinates)
    /// fully into the viewport, moving as little as possible.
    pub fn offset_revealing(&self, top: f64, bottom: f64) -> f64 {
        let view_bottom = self.viewport_top + self.viewport;
        let shift = if top < self.viewport_top {
            top - self.viewport_top
        } else if bottom > view_bottom {
            (bottom - view_bottom).min(top - self.viewport_top)
        } else {
            0.0
        };
        (self.offset + shift).clamp(0.0, self.max_offset())
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
    /// State gates a gated layout's nodes and hit regions index.
    pub gates: Vec<StateGate>,
}
