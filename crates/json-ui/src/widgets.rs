//! Engine-driven control behaviour that the templates only name: which state
//! child of a button/toggle/edit box/slider shows, where a slider's box sits and
//! how much of its progress bar is revealed, and how a scroll view offsets its
//! content and sizes its scrollbar box. The templates supply the child names
//! (`default_control`, `checked_hover_control`, `scroll_content`, …); the state
//! comes from the bound `#` values and the caller's [`ViewState`].

use serde_json::Value;

use crate::layout::Rect;
use crate::state::{ScrollMetrics, ViewState};
use crate::tree::ResolvedControl;

mod states;

pub(crate) use states::{state_index, state_targets};

/// The slider bag value holding its box's selected (indent) state.
pub(crate) use crate::component::SLIDER_BOX_SELECTED;

fn prop_str<'a>(control: &'a ResolvedControl, key: &str) -> Option<&'a str> {
    control
        .properties
        .get(key)
        .and_then(Value::as_str)
        .filter(|name| !name.is_empty())
}

/// A bound boolean: literal bool, `"true"`/`"false"`, or `None`.
pub(crate) fn bound_bool(control: &ResolvedControl, key: &str) -> Option<bool> {
    match control.properties.get(key)? {
        Value::Bool(flag) => Some(*flag),
        Value::String(text) if text == "true" => Some(true),
        Value::String(text) if text == "false" => Some(false),
        _ => None,
    }
}

pub(crate) fn bound_number(control: &ResolvedControl, key: &str) -> Option<f64> {
    match control.properties.get(key)? {
        Value::Number(number) => number.as_f64(),
        Value::String(text) => text.parse().ok(),
        _ => None,
    }
}

/// Whether the control accepts input: `enabled`/`#enabled` false locks it.
pub(crate) fn enabled(control: &ResolvedControl) -> bool {
    bound_bool(control, "#enabled")
        .or_else(|| bound_bool(control, "enabled"))
        .unwrap_or(true)
}

/// A toggle's checked state: the bound `#toggle_state`, else its default.
pub(crate) fn toggle_checked(control: &ResolvedControl) -> bool {
    bound_bool(control, "#toggle_state")
        .or_else(|| bound_bool(control, "toggle_default_state"))
        .unwrap_or(false)
}

/// A slider's normalized position `0..=1`: a step slider's `#slider_value` is the
/// step index over `#slider_steps`, a continuous slider's value is already a
/// fraction.
fn slider_fraction(control: &ResolvedControl) -> Option<f64> {
    if control.control_type.as_deref() != Some("slider") {
        return None;
    }
    let value = bound_number(control, "#slider_value").unwrap_or(0.0);
    let steps = bound_number(control, "#slider_steps")
        .or_else(|| bound_number(control, "slider_steps"))
        .unwrap_or(1.0);
    let fraction = if steps > 1.0 {
        value / (steps - 1.0)
    } else {
        value
    };
    let fraction = fraction.clamp(0.0, 1.0);
    Some(if bound_bool(control, "slider_inverted") == Some(true) {
        1.0 - fraction
    } else {
        fraction
    })
}

/// A slider being laid out: where its box travels and which children it clips.
pub(crate) struct SliderFrame {
    pub fraction: f64,
    /// `slider_box_control`, `progress_control`, `progress_hover_control`.
    pub names: [Option<String>; 3],
    pub rect: Rect,
    pub vertical: bool,
}

impl SliderFrame {
    pub fn open(control: &ResolvedControl, rect: Rect) -> Option<Self> {
        Some(Self {
            fraction: slider_fraction(control)?,
            names: [
                prop_str(control, "slider_box_control").map(str::to_owned),
                prop_str(control, "progress_control").map(str::to_owned),
                prop_str(control, "progress_hover_control").map(str::to_owned),
            ],
            rect,
            vertical: prop_str(control, "slider_direction") == Some("vertical"),
        })
    }

    /// The box's rect: its centre travels the full slider along its axis.
    pub fn place_box(&self, box_rect: Rect) -> Rect {
        let track = self.rect;
        if self.vertical {
            return Rect::new(
                box_rect.x,
                track.y + track.h * self.fraction - box_rect.h * 0.5,
                box_rect.w,
                box_rect.h,
            );
        }
        Rect::new(
            track.x + track.w * self.fraction - box_rect.w * 0.5,
            box_rect.y,
            box_rect.w,
            box_rect.h,
        )
    }
}

/// A panel holding a `dropdown`: the dropdown's name, its `dropdown_area` and
/// its content sibling's name (`DropdownComponent`).
pub(crate) fn dropdown_area(control: &ResolvedControl) -> Option<(String, String, String)> {
    control.children.iter().find_map(|child| {
        if child.control_type.as_deref() != Some("dropdown") {
            return None;
        }
        let area = prop_str(child, "dropdown_area")?;
        let content = prop_str(child, "dropdown_content_control").unwrap_or("dropdown_content");
        Some((child.name.clone(), area.to_owned(), content.to_owned()))
    })
}

/// The content's top as `DropdownComponent::_positionContent` places it:
/// level with the dropdown, raised to end inside the area, never above it,
/// and centred on the area when taller than it.
pub(crate) fn dropdown_content_top(dropdown: Rect, area: Rect, content_height: f64) -> f64 {
    if area.h <= content_height {
        return area.y + area.h * 0.5 - content_height * 0.5;
    }
    let raised = if area.h + area.y < content_height + dropdown.y {
        area.h + area.y - content_height
    } else {
        dropdown.y
    };
    raised.max(area.y)
}

/// The live scroll view being laid out: which descendants are its content and box.
pub(crate) struct ScrollFrame {
    pub key: String,
    pub content: String,
    pub bar_box: String,
    pub requested: f64,
    pub always_visible: bool,
    pub speed: f64,
    pub metrics: Option<ScrollMetrics>,
}

impl ScrollFrame {
    pub fn open(control: &ResolvedControl, key: &str, state: &ViewState) -> Option<Self> {
        if control.control_type.as_deref() != Some("scroll_view") {
            return None;
        }
        Some(Self {
            key: key.to_owned(),
            content: prop_str(control, "scroll_content")?.to_owned(),
            bar_box: prop_str(control, "scrollbar_box")
                .unwrap_or("box")
                .to_owned(),
            // A view that jumps to its end on update opens at the end until the caller scrolls it.
            requested: state.scroll.get(key).copied().unwrap_or(
                if bound_bool(control, "jump_to_bottom_on_update").unwrap_or(false) {
                    f64::INFINITY
                } else {
                    0.0
                },
            ),
            always_visible: bound_bool(control, "scrollbar_always_visible").unwrap_or(false),
            speed: bound_number(control, "scroll_speed").unwrap_or(15.0),
            metrics: None,
        })
    }

    /// Shift the content child up by the clamped offset, recording the extents.
    pub fn place_content(&mut self, viewport: Rect, content: Rect) -> Rect {
        let max = (content.h - viewport.h).max(0.0);
        let offset = self.requested.clamp(0.0, max);
        self.metrics = Some(ScrollMetrics {
            offset,
            content: content.h,
            viewport: viewport.h,
            viewport_top: viewport.y,
            track: None,
            thumb: None,
            speed: self.speed,
        });
        // Overflowing content scrolls from the viewport's top whatever its anchor.
        let top = if max > 0.0 {
            viewport.y - offset
        } else {
            content.y
        };
        Rect::new(content.x, top, content.w, content.h)
    }

    /// Size and position the scrollbar box inside `track`; `None` hides it.
    pub fn place_box(&mut self, track: Rect, box_rect: Rect) -> Option<Rect> {
        let metrics = self.metrics.as_mut()?;
        metrics.track = Some([track.x, track.y, track.w, track.h]);
        let max = metrics.max_offset();
        if max <= 0.0 && !self.always_visible {
            return None;
        }
        let ratio = if metrics.content > 0.0 {
            (metrics.viewport / metrics.content).clamp(0.0, 1.0)
        } else {
            1.0
        };
        let height = (track.h * ratio).max(box_rect.w.min(track.h));
        let travel = (track.h - height).max(0.0);
        let fraction = if max > 0.0 { metrics.offset / max } else { 0.0 };
        let placed = Rect::new(box_rect.x, track.y + travel * fraction, box_rect.w, height);
        metrics.thumb = Some([placed.x, placed.y, placed.w, placed.h]);
        Some(placed)
    }
}
