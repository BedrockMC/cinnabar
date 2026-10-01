//! A `scroll_view` as vanilla's ScrollViewComponent lays it out: its five named
//! descendants (content, viewport, track, box, box-and-track panel), the offset
//! placing the content, the box sized and moved along the track, the panel
//! hidden when the content fits, and the property-bag feedback it publishes.

use crate::layout::Rect;
use crate::state::{ScrollMetrics, ViewState};
use crate::tree::ResolvedControl;
use crate::widgets::{bound_bool, bound_number, prop_str};

/// Smallest box length as a fraction of the track (1.26.50 `_updateScrollBoxSize`).
const MIN_BOX_RATIO: f64 = 0.1;
/// Offsets snap to eighths of a pixel before placing the content.
const OFFSET_STEPS: f64 = 8.0;
/// Touch overscroll allowed past either end, as a fraction of the viewport.
pub(crate) const OVERSCROLL: f64 = 0.25;
/// `#scrollbar_hit_bottom` latches within this many pixels of the end.
const HIT_BOTTOM_EPSILON: f64 = 0.1;

/// `draggable` on a layout component (`ui::Draggable`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Draggable {
    #[default]
    NotDraggable,
    Horizontal,
    Vertical,
    Both,
}

impl Draggable {
    pub(crate) fn of(control: &ResolvedControl) -> Self {
        match prop_str(control, "draggable") {
            Some("horizontal") => Draggable::Horizontal,
            Some("vertical") => Draggable::Vertical,
            Some("both") => Draggable::Both,
            _ => Draggable::NotDraggable,
        }
    }
}

/// Which of the five references the scroll view resolves under it.
#[derive(Default)]
struct Found {
    content: bool,
    viewport: bool,
    track: bool,
    bar_box: bool,
    panel: bool,
}

/// The live scroll view being laid out.
pub(crate) struct ScrollFrame {
    pub key: String,
    content: String,
    viewport_name: String,
    track_name: String,
    bar_box: String,
    panel: String,
    requested: f64,
    /// A retained touch motion may carry the offset past either end.
    moving: bool,
    /// `#force_scroll_to_end`, or a new extent under `jump_to_bottom_on_update`.
    pin_end: bool,
    /// The extent the caller last settled, for `jump_to_bottom_on_update`.
    known_extent: Option<f64>,
    always_visible: bool,
    touch_mode: bool,
    found: Found,
    viewport: Option<Rect>,
    track: Option<Rect>,
    /// The box's axis and whether the panel/box follow the content extent.
    box_drag: Draggable,
    box_placed: bool,
    panel_placed: bool,
    /// Box children's alpha while a touch scrollbar fades; `Some(0)` hides it.
    touch_fade: Option<f32>,
    pub metrics: Option<ScrollMetrics>,
}

/// What the frame does to one descendant as it is placed.
pub(crate) enum Placement {
    Keep,
    Move(Rect),
    /// The scrollbar box at a new size: re-anchor it, then shift it by the delta.
    Box {
        size: [f64; 2],
        delta: [f64; 2],
        fade: Option<f32>,
    },
    Hide,
}

impl ScrollFrame {
    pub fn open(control: &ResolvedControl, key: &str, state: &ViewState) -> Option<Self> {
        if control.control_type.as_deref() != Some("scroll_view") {
            return None;
        }
        let name = |property: &str| prop_str(control, property).unwrap_or("").to_owned();
        let mut frame = Self {
            key: key.to_owned(),
            content: name("scroll_content"),
            viewport_name: name("scroll_view_port"),
            track_name: name("scrollbar_track"),
            bar_box: name("scrollbar_box"),
            panel: name("scroll_box_and_track_panel"),
            requested: 0.0,
            moving: false,
            pin_end: false,
            known_extent: None,
            always_visible: bound_bool(control, "scrollbar_always_visible").unwrap_or(false),
            touch_mode: bound_bool(control, "touch_mode").unwrap_or(false),
            found: Found::default(),
            viewport: None,
            track: None,
            box_drag: Draggable::NotDraggable,
            box_placed: false,
            panel_placed: false,
            touch_fade: None,
            metrics: None,
        };
        frame.scan(control);
        let retained = state.scroll_state.get(key);
        frame.requested = state.scroll_offset(key);
        frame.moving = retained.is_some_and(|retained| retained.motion.is_some());
        // A caller that never settles keeps its own offset once it sets one.
        frame.known_extent = retained
            .and_then(|retained| retained.extent)
            .or_else(|| state.scroll.contains_key(key).then_some(f64::NAN));
        if frame.touch_mode {
            frame.touch_fade = retained.and_then(|retained| retained.bar_fade);
        }
        frame.pin_end = bound_bool(control, "#force_scroll_to_end") == Some(true);
        let metrics = ScrollMetrics {
            speed: bound_number(control, "scroll_speed").unwrap_or(1.0),
            gesture: bound_bool(control, "#gesture_control_enabled")
                .or_else(|| bound_bool(control, "gesture_control_enabled"))
                .unwrap_or(false),
            always_handle_scrolling: bound_bool(control, "always_handle_scrolling")
                .unwrap_or(false),
            touch_mode: frame.touch_mode,
            allow_scroll_when_fits: bound_bool(control, "allow_scroll_even_when_content_fits")
                .unwrap_or(true),
            jump_to_end: bound_bool(control, "jump_to_bottom_on_update").unwrap_or(false),
            track_button: prop_str(control, "scrollbar_track_button").map(str::to_owned),
            touch_button: prop_str(control, "scrollbar_touch_button").map(str::to_owned),
            scrolled_to_end: true,
            hit_bottom: retained.is_some_and(|retained| retained.hit_bottom),
            ..ScrollMetrics::default()
        };
        frame.metrics = Some(metrics);
        Some(frame)
    }

    /// Record which references resolve, skipping the content's subtree and
    /// nested scroll views, which own their own names.
    fn scan(&mut self, control: &ResolvedControl) {
        for child in &control.children {
            let name = child.name.as_str();
            let found = &mut self.found;
            found.viewport |= !self.viewport_name.is_empty() && name == self.viewport_name;
            found.track |= !self.track_name.is_empty() && name == self.track_name;
            found.panel |= !self.panel.is_empty() && name == self.panel;
            if !self.bar_box.is_empty() && name == self.bar_box && !found.bar_box {
                found.bar_box = true;
                self.box_drag = Draggable::of(child);
            }
            if !self.content.is_empty() && name == self.content {
                found.content = true;
                continue;
            }
            if child.control_type.as_deref() != Some("scroll_view") {
                self.scan(child);
            }
        }
    }

    /// Vanilla scrolls only with its content, viewport, track and box all resolved.
    fn scrolls(&self) -> bool {
        self.found.content && self.found.viewport && self.found.track && self.found.bar_box
    }

    /// The panel and box follow the extent only for a one-axis box under a named panel.
    fn sizes_box(&self) -> bool {
        self.scrolls()
            && self.found.panel
            && matches!(self.box_drag, Draggable::Horizontal | Draggable::Vertical)
    }

    /// Whether the content axis is horizontal (a horizontally draggable box).
    fn horizontal(&self) -> bool {
        self.box_drag == Draggable::Horizontal
    }

    /// Place `child` (just given `rect`); `anchor` is its `anchor_from` fraction.
    pub fn place(&mut self, child: &ResolvedControl, rect: Rect, anchor: [f64; 2]) -> Placement {
        let name = child.name.as_str();
        if self.viewport.is_none() && name == self.viewport_name && self.found.viewport {
            self.viewport = Some(rect);
            if let Some(metrics) = self.metrics.as_mut() {
                metrics.viewport_rect = Some([rect.x, rect.y, rect.w, rect.h]);
            }
        }
        if self.track.is_none() && name == self.track_name && self.found.track {
            self.track = Some(rect);
            if let Some(metrics) = self.metrics.as_mut() {
                metrics.track = Some([rect.x, rect.y, rect.w, rect.h]);
            }
        }
        if name == self.content && self.found.content {
            return self.place_content(rect, anchor);
        }
        if !self.panel_placed && name == self.panel && self.found.panel {
            self.panel_placed = true;
            return self.place_panel();
        }
        if !self.box_placed && name == self.bar_box && self.found.bar_box {
            self.box_placed = true;
            return self.place_box(rect);
        }
        Placement::Keep
    }

    fn place_content(&mut self, content: Rect, anchor: [f64; 2]) -> Placement {
        let (Some(viewport), true) = (self.viewport, self.scrolls()) else {
            return Placement::Keep;
        };
        let horizontal = self.horizontal();
        let Some(metrics) = self.metrics.as_mut() else {
            return Placement::Keep;
        };
        if metrics.content_rect.is_some() {
            return Placement::Keep;
        }
        let overflow = [content.w - viewport.w, content.h - viewport.h];
        let (extent, size, start) = if horizontal {
            (content.w, viewport.w, viewport.x)
        } else {
            (content.h, viewport.h, viewport.y)
        };
        let max = (extent - size).max(0.0);
        let slack = if self.moving { size * OVERSCROLL } else { 0.0 };
        let grew = metrics.jump_to_end
            && self
                .known_extent
                .is_none_or(|known| !known.is_nan() && known != max);
        let offset = if self.pin_end || grew {
            max
        } else {
            self.requested.clamp(-slack, max + slack)
        };
        let snapped = (offset * OFFSET_STEPS).trunc() / OFFSET_STEPS;
        metrics.offset = offset;
        metrics.content = extent;
        metrics.viewport = size;
        metrics.viewport_top = start;
        metrics.horizontal = horizontal;
        metrics.content_rect = Some([content.x, content.y, content.w, content.h]);
        metrics.scrolled_to_end = max <= snapped;
        let overflow_y = overflow[1];
        let reached = snapped != 0.0 && overflow_y <= snapped;
        if (!horizontal && (snapped - max).abs() < HIT_BOTTOM_EPSILON) || reached {
            metrics.hit_bottom = true;
        }
        // Content on the far edge keeps its overflow there, as `setOffsetDelta` does.
        let far = |fraction: f64, overflow: f64| fraction >= 1.0 && overflow > 0.0;
        let mut delta = [0.0, 0.0];
        let (along, across) = if horizontal { (0, 1) } else { (1, 0) };
        delta[along] = if far(anchor[along], overflow[along]) {
            overflow[along] - snapped
        } else {
            -snapped
        };
        if far(anchor[across], overflow[across]) {
            delta[across] = overflow[across];
        }
        Placement::Move(Rect::new(
            content.x + delta[0],
            content.y + delta[1],
            content.w,
            content.h,
        ))
    }

    /// Whether the content fits its viewport, `None` before the content is placed.
    fn fits(&self) -> Option<bool> {
        let metrics = self.metrics.as_ref()?;
        metrics.content_rect?;
        if metrics.content <= 0.0 {
            return Some(true);
        }
        let ratio = metrics.viewport / metrics.content;
        Some(ratio.max(MIN_BOX_RATIO) == 1.0 || ratio > 1.0)
    }

    fn place_panel(&mut self) -> Placement {
        let shown = match (self.sizes_box(), self.fits()) {
            (true, Some(true)) => {
                if let Some(metrics) = self.metrics.as_mut() {
                    metrics.hit_bottom = true;
                }
                self.always_visible
            }
            _ => true,
        };
        if let Some(metrics) = self.metrics.as_mut() {
            metrics.bar_visible = Some(shown);
        }
        if shown {
            Placement::Keep
        } else {
            Placement::Hide
        }
    }

    fn place_box(&mut self, rect: Rect) -> Placement {
        let fade = self.touch_fade;
        if fade == Some(0.0) {
            if let Some(metrics) = self.metrics.as_mut() {
                metrics.bar_visible = Some(false);
            }
            return Placement::Hide;
        }
        let (Some(track), true) = (self.track, self.scrolls()) else {
            return Placement::Keep;
        };
        let sizes = self.sizes_box();
        let horizontal = self.horizontal();
        let Some(metrics) = self.metrics.as_mut() else {
            return Placement::Keep;
        };
        // A box under a hidden panel is neither sized nor drawn.
        if metrics.content_rect.is_none() || metrics.bar_visible == Some(false) {
            return Placement::Keep;
        }
        let mut size = [rect.w, rect.h];
        if sizes {
            let ratio = if metrics.content > 0.0 {
                (metrics.viewport / metrics.content).clamp(MIN_BOX_RATIO, 1.0)
            } else {
                1.0
            };
            let (index, track_size) = if horizontal {
                (0, track.w)
            } else {
                (1, track.h)
            };
            if track_size > 0.0 {
                size[index] = (ratio * track_size).ceil();
            }
        }
        let max = metrics.max_offset();
        let snapped = (metrics.offset * OFFSET_STEPS).trunc() / OFFSET_STEPS;
        let fraction = if max > 0.5 { snapped / max } else { 1.0 };
        let mut delta = [0.0, 0.0];
        if horizontal {
            delta[0] = (track.w - size[0]) * fraction;
        } else {
            delta[1] = (track.h - size[1]) * fraction;
        }
        metrics.box_drag = self.box_drag;
        if fade.is_some() {
            metrics.bar_visible = Some(true);
        }
        Placement::Box { size, delta, fade }
    }

    /// Whether the content has taken its scrolled place (culling may start).
    pub fn content_placed(&self) -> bool {
        self.metrics
            .as_ref()
            .is_some_and(|metrics| metrics.content_rect.is_some())
    }

    /// Record where the box finally landed.
    pub fn placed_box(&mut self, rect: Rect) {
        if let Some(metrics) = self.metrics.as_mut() {
            metrics.thumb = Some([rect.x, rect.y, rect.w, rect.h]);
        }
    }

    /// The finished metrics; `None` when the view never scrolled.
    pub fn finish(self) -> Option<ScrollMetrics> {
        self.metrics
            .filter(|metrics| metrics.content_rect.is_some())
    }
}
