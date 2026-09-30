//! Placement: a sized control's rect from its anchors and offset, and the
//! offset animation measured the same way.

use serde_json::Value;

use super::{Axis, AxisContext, LayoutEnv, Rect, ResolvedControl, axis_index, axis_of, pixels_or};
use crate::anim::{Inherited, Motion, SLIDE_KEY, Slide};
use crate::expr;

/// The child's rect from its resolved size and anchor/offset within `parent_rect`:
/// its `anchor_to` point lands on the parent's `anchor_from` point.
pub(super) fn place_by_anchor(
    control: &ResolvedControl,
    parent_rect: Rect,
    size: [f64; 2],
    _env: &LayoutEnv,
) -> Rect {
    let from = anchor_from(control);
    let to = anchor_to(control);
    let off = offset(control, parent_rect, size);
    let x = parent_rect.x + parent_rect.w * anchor_frac(from, Axis::X)
        - size[0] * anchor_frac(to, Axis::X)
        + off[0];
    let y = parent_rect.y + parent_rect.h * anchor_frac(from, Axis::Y)
        - size[1] * anchor_frac(to, Axis::Y)
        + off[1];
    Rect::new(x, y, size[0], size[1])
}

/// The static `offset`: `%` of the parent, `%x`/`%y` of the control's own size.
pub(super) fn offset(control: &ResolvedControl, parent_rect: Rect, size: [f64; 2]) -> [f64; 2] {
    control
        .properties
        .get("offset")
        .map_or([0.0; 2], |pair| offset_pixels(pair, parent_rect, size))
}

/// An `[x, y]` offset pair in pixels; anything else is no offset.
fn offset_pixels(pair: &Value, parent_rect: Rect, size: [f64; 2]) -> [f64; 2] {
    let Value::Array(items) = pair else {
        return [0.0; 2];
    };
    if items.len() < 2 {
        return [0.0; 2];
    }
    let axis_value = |index: usize, axis: Axis| {
        let ctx = AxisContext {
            parent: axis_of(parent_rect, axis),
            own_width: Some(size[0]),
            own_height: Some(size[1]),
            ..AxisContext::default()
        };
        expr::length_from_value(&items[index])
            .map(|len| pixels_or(len.eval(&ctx), 0.0))
            .unwrap_or(0.0)
    };
    [axis_value(0, Axis::X), axis_value(1, Axis::Y)]
}

/// The control's `offset` animation in pixels, measured like its static offset.
pub(super) fn motion(
    control: &ResolvedControl,
    parent_rect: Rect,
    size: [f64; 2],
    inherited: &Inherited,
) -> Option<Motion> {
    let slide: Slide = serde_json::from_value(control.properties.get(SLIDE_KEY)?.clone()).ok()?;
    let (born, clock) = inherited.timing();
    let rest = offset(control, parent_rect, size);
    Some(slide.motion(rest, born, clock, |pair| match pair {
        Value::Null => rest,
        pair => offset_pixels(pair, parent_rect, size),
    }))
}

pub(super) fn anchor_from(control: &ResolvedControl) -> [f64; 2] {
    anchor(control, "anchor_from")
}

pub(super) fn anchor_to(control: &ResolvedControl) -> [f64; 2] {
    anchor(control, "anchor_to")
}

/// Fractional anchor point `[fx, fy]`, defaulting to `center`.
fn anchor(control: &ResolvedControl, key: &str) -> [f64; 2] {
    // `left`/`right` and `top`/`bottom` appear as either half of a name
    // (`top_left`, `left_middle`), so match on membership, not position.
    let name = control
        .properties
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or("center");
    let fx = if name.contains("left") {
        0.0
    } else if name.contains("right") {
        1.0
    } else {
        0.5
    };
    let fy = if name.contains("top") {
        0.0
    } else if name.contains("bottom") {
        1.0
    } else {
        0.5
    };
    [fx, fy]
}

pub(super) fn anchor_frac(point: [f64; 2], axis: Axis) -> f64 {
    point[axis_index(axis)]
}
