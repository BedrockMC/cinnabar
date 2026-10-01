//! Placement: a sized control's rect from its anchors and offset, and the
//! offset animation measured the same way.

use serde_json::Value;

use super::{Axis, LayoutEnv, Rect, ResolvedControl, axis_index, measure};
use crate::anim::{Inherited, Motion, SLIDE_KEY, Slide};
use crate::expr::{self, AxisContext, Length};

/// The child's rect from its resolved size and anchor/offset within `parent_rect`:
/// its `anchor_to` point lands on the parent's `anchor_from` point.
pub(super) fn place_by_anchor(
    control: &ResolvedControl,
    parent_rect: Rect,
    size: [f64; 2],
    siblings: [f64; 2],
    env: &LayoutEnv,
) -> Rect {
    let from = anchor_from(control);
    let to = anchor_to(control);
    let off = offset(control, parent_rect, size, siblings, env);
    let x = parent_rect.x + parent_rect.w * from[0] - size[0] * to[0] + off[0];
    let y = parent_rect.y + parent_rect.h * from[1] - size[1] * to[1] + off[1];
    Rect::new(x, y, size[0], size[1])
}

/// The static `offset` in pixels.
fn offset(
    control: &ResolvedControl,
    parent_rect: Rect,
    size: [f64; 2],
    siblings: [f64; 2],
    env: &LayoutEnv,
) -> [f64; 2] {
    control.properties.get("offset").map_or([0.0; 2], |pair| {
        offset_pixels(control, pair, parent_rect, size, siblings, env)
    })
}
/// An `[x, y]` offset pair in pixels, its units read like size units (`%` of the
/// parent, `%x`/`%y` own size, `%c`/`%cm` children, `%sm` siblings). An axis
/// that is not an expression (`default`, `fill`) adds no offset.
fn offset_pixels(
    control: &ResolvedControl,
    pair: &Value,
    parent_rect: Rect,
    size: [f64; 2],
    siblings: [f64; 2],
    env: &LayoutEnv,
) -> [f64; 2] {
    let Value::Array(items) = pair else {
        return [0.0; 2];
    };
    let children = measure::children(control, env, [Some(size[0]), Some(size[1])]);
    let axis_value = |axis: Axis| {
        let index = axis_index(axis);
        let Some(Length::Terms(terms)) = items.get(index).map(expr::length_from_value) else {
            return 0.0;
        };
        let ctx = AxisContext {
            parent: [parent_rect.w, parent_rect.h][index],
            own_width: Some(size[0]),
            own_height: Some(size[1]),
            children: Some(children.content[index]),
            children_max: Some(children.maximum[index]),
            sibling_max: Some(siblings[index]),
            natural: None,
        };
        Length::Terms(terms).eval_pixels(&ctx)
    };
    [axis_value(Axis::X), axis_value(Axis::Y)]
}

/// The control's `offset` animation in pixels, measured like its static offset.
pub(super) fn motion(
    control: &ResolvedControl,
    parent_rect: Rect,
    size: [f64; 2],
    inherited: &Inherited,
    env: &LayoutEnv,
) -> Option<Motion> {
    let slide: Slide = serde_json::from_value(control.properties.get(SLIDE_KEY)?.clone()).ok()?;
    let (born, clock) = inherited.timing();
    let rest = offset(control, parent_rect, size, [0.0; 2], env);
    Some(slide.motion(rest, born, clock, |pair| match pair {
        Value::Null => rest,
        pair => offset_pixels(control, pair, parent_rect, size, [0.0; 2], env),
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
