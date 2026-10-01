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
    /// Scroll view key → its maximum offset last layout, which
    /// `jump_to_bottom_on_update` compares against (see [`ViewState::remember`]).
    pub scroll_max: BTreeMap<String, f64>,
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

    /// Keep each scroll view's applied offset and maximum from `report` for the
    /// next layout.
    pub fn remember(&mut self, report: &LayoutReport) {
        for (key, metrics) in &report.scrolls {
            self.scroll.insert(key.clone(), metrics.offset);
            self.scroll_max.insert(key.clone(), metrics.max_offset());
        }
    }
}

/// A scroll view's measured extents after layout, along its scrolling axis.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ScrollMetrics {
    /// The clamped offset actually applied.
    pub offset: f64,
    pub content: f64,
    pub viewport: f64,
    /// The viewport's leading edge (virtual px), for scrolling a control into view.
    pub viewport_top: f64,
    /// The viewport rect `[x, y, w, h]`, where the wheel reaches the view.
    pub port: Option<[f64; 4]>,
    /// The scrollbar track rect `[x, y, w, h]`, when the template has one.
    pub track: Option<[f64; 4]>,
    /// The drawn scrollbar box rect `[x, y, w, h]`, when it is sized.
    pub thumb: Option<[f64; 4]>,
    /// `scroll_speed`: pixels per wheel step (1 when unset).
    pub speed: f64,
    /// Whether the content scrolls along x (its `draggable` is horizontal).
    pub horizontal: bool,
    /// `#scrolled_to_end`: the offset reaches the maximum.
    pub scrolled_to_end: bool,
    /// `#scrollbar_hit_bottom`: the content fits or its bottom is reached.
    pub hit_bottom: bool,
    /// `#scroll_bar_visible`: the bar panel shows.
    pub bar_visible: bool,
    /// `always_handle_scrolling`: wheel input reaches the view wherever the pointer is.
    pub always_handle: bool,
    pub touch_mode: bool,
    /// `gesture_control_enabled`: pressing and dragging the content scrolls it.
    pub gesture_control: bool,
    /// `allow_scroll_even_when_content_fits`.
    pub scroll_when_fits: bool,
    /// The track's `button.menu_select` press routes to `scrollbar_track_button`.
    pub track_clicks: bool,
    /// The view's own press routes to `scrollbar_touch_button`, starting a drag.
    pub touch_drags: bool,
}

/// The client reads its wheel sensitivity once, from the first view scrolled.
static WHEEL_SENSITIVITY: std::sync::OnceLock<f64> = std::sync::OnceLock::new();

impl ScrollMetrics {
    pub fn max_offset(&self) -> f64 {
        (self.content - self.viewport).max(0.0)
    }

    fn clamp(&self, offset: f64) -> f64 {
        offset.clamp(0.0, self.max_offset())
    }

    /// The offset after `notches` wheel steps (positive scrolls toward the top):
    /// the first view scrolled fixes the sensitivity, and a step up is 120/127 of
    /// one down is 120/128, the client's mouse byte scaling. A horizontal view
    /// ignores the vertical wheel.
    pub fn wheel_target(&self, notches: f64) -> f64 {
        if self.horizontal {
            return self.offset;
        }
        let sensitivity = *WHEEL_SENSITIVITY.get_or_init(|| self.speed);
        let byte = if notches > 0.0 {
            120.0 / 127.0
        } else {
            120.0 / 128.0
        };
        self.clamp(self.offset - sensitivity * notches * byte)
    }

    /// The offset after the pointer moves `delta` along the axis while holding the
    /// box: content pixels per track pixel.
    pub fn thumb_drag_target(&self, delta: f64) -> f64 {
        let track = self.track_extent();
        if track <= 0.0 {
            return self.offset;
        }
        self.clamp(self.offset + delta * self.content / track)
    }

    /// The offset a click at `point` on the track jumps to: the clicked fraction
    /// of the content, less half a viewport.
    pub fn track_target(&self, point: [f64; 2]) -> f64 {
        let Some(track) = self.track else {
            return self.offset;
        };
        let (start, length, at) = if self.horizontal {
            (track[0], track[2], point[0])
        } else {
            (track[1], track[3], point[1])
        };
        let fraction = if length <= 0.5 {
            1.0
        } else {
            (at - start) / length
        };
        self.clamp(fraction * self.content - 0.5 * self.viewport)
    }

    /// Whether pressing and moving over the content drags it: the touch button
    /// is mapped, gestures are on, and the content overflows vertically (the
    /// client compares heights whatever the axis) unless it may scroll anyway.
    pub fn drags_content(&self) -> bool {
        let overflows = if self.horizontal {
            self.port.is_some_and(|port| port[3] < self.content)
        } else {
            self.viewport < self.content
        };
        self.touch_drags && self.gesture_control && (overflows || self.scroll_when_fits)
    }

    fn track_extent(&self) -> f64 {
        self.track.map_or(
            0.0,
            |track| if self.horizontal { track[2] } else { track[3] },
        )
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
        self.clamp(self.offset + shift)
    }
}

/// Side results gathered while laying out: every scroll view's metrics.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct LayoutReport {
    pub scrolls: BTreeMap<String, ScrollMetrics>,
}

/// A content drag under the client's scroll dynamics: while held the offset
/// springs toward the dragged target (velocity sampled over 0.05 s windows);
/// released, it flings, decays, and springs back inside `[0, max]`, never
/// travelling more than a quarter viewport past either end.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ScrollDynamics {
    pub position: f64,
    velocity: f64,
    target: f64,
    target_velocity: f64,
    dragging: bool,
    /// Pointer travel in the current and previous sample windows.
    window: [f64; 2],
    window_age: f64,
    /// Total pointer travel this drag; a short one is a tap.
    distance: f64,
}

/// A drag shorter than this is a tap that passes through to the content.
const TAP_DISTANCE: f64 = 5.0;
const WINDOW: f64 = 0.05;
const STEP: f64 = 0.01;

impl ScrollDynamics {
    /// Start a drag at `offset`.
    pub fn press(&mut self, offset: f64) {
        if !self.dragging {
            self.position = offset;
            self.target = offset;
            self.window = [0.0; 2];
            self.window_age = 0.0;
            self.distance = 0.0;
        }
        self.dragging = true;
    }

    /// The pointer moved `delta` along the axis; the content follows it.
    pub fn drag(&mut self, delta: f64) {
        self.target -= delta;
        self.window[0] -= delta;
        self.distance += delta.abs();
    }

    /// Let go, flinging at the faster of the tracked and the last window's
    /// velocity. Returns whether it was a tap.
    pub fn release(&mut self) -> bool {
        if self.dragging {
            let recent = self.window[0] / WINDOW;
            self.velocity = if recent.abs() > self.target_velocity.abs() {
                recent
            } else {
                self.target_velocity
            };
            self.dragging = false;
        }
        self.distance <= TAP_DISTANCE
    }

    /// Whether the offset still moves.
    pub fn active(&self) -> bool {
        self.dragging || self.velocity.abs() > 1.0
    }

    /// Advance `dt` seconds (at most a quarter second) within `[0, max]`, with
    /// `viewport` bounding the overscroll; returns the new offset.
    pub fn tick(&mut self, dt: f64, max: f64, viewport: f64) -> f64 {
        let mut remaining = dt.clamp(0.0, 0.25);
        let over = (viewport * 0.25).max(0.0);
        if self.dragging {
            self.window_age += remaining;
            if self.window_age >= WINDOW {
                self.target_velocity = (self.window[0] + self.window[1]) / (2.0 * WINDOW);
                self.window = [0.0, self.window[0]];
                self.window_age = 0.0;
            }
        } else {
            self.target = self.position;
        }
        while remaining > 0.0 {
            let h = remaining.min(STEP);
            let mut accel = if (0.0..=max).contains(&self.position) {
                -self.velocity.signum() * (self.velocity.abs() / h).min(200.0)
            } else {
                0.0
            };
            if self.position < 0.0 || self.position > max {
                let bound = if self.position < 0.0 { 0.0 } else { max };
                accel = -34.641 * self.velocity + 300.0 * (bound - self.position);
            }
            if self.dragging {
                accel += 63.2456 * (self.target_velocity - self.velocity)
                    + 1000.0 * (self.target - self.position);
            }
            self.velocity = (self.velocity + h * accel).clamp(-2000.0, 2000.0);
            self.position += h * self.velocity;
            if self.position < -over {
                self.position = -over;
                self.velocity = self.velocity.max(0.0);
            } else if self.position > max + over {
                self.position = max + over;
                self.velocity = self.velocity.min(0.0);
            }
            remaining -= h;
        }
        self.position
    }
}
