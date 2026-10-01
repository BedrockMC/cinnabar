//! Placement: a sized control's rect from its anchors and offset, and its
//! animations' offset and size ends measured the same way.

use serde_json::Value;

use super::{Axis, AxisContext, LayoutEnv, Rect, ResolvedControl, axis_index, axis_of, pixels_or};
use std::sync::Arc;

use crate::anim::{AnimGraph, AnimKind, ControlAnims, GRAPH_KEY, Inherited, NodeAnim};
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

/// The control's animations with offset and size ends in pixels, `%` of the
/// parent and `%x`/`%y` of the control's own size.
pub(super) fn control_anims(
    control: &ResolvedControl,
    key: &str,
    rect: Rect,
    parent_rect: Rect,
    inherited: &Inherited,
) -> Option<Arc<ControlAnims>> {
    let mut graph: AnimGraph =
        serde_json::from_value(control.properties.get(GRAPH_KEY)?.clone()).ok()?;
    let size = [rect.w, rect.h];
    let rest_offset = offset(control, parent_rect, size);
    for node in &mut graph.nodes {
        let ends = match node.kind {
            AnimKind::Offset => {
                [&node.from_expr, &node.to_expr].map(|pair| offset_pixels(pair, parent_rect, size))
            }
            AnimKind::Size => [&node.from_expr, &node.to_expr].map(|pair| match pair {
                Value::Array(_) => offset_pixels(pair, parent_rect, size),
                _ => size,
            }),
            _ => continue,
        };
        node.from = [ends[0][0] as f32, ends[0][1] as f32, 0.0, 0.0];
        node.to = [ends[1][0] as f32, ends[1][1] as f32, 0.0, 0.0];
    }
    let context = inherited.own(control);
    let wait_scale = control
        .properties
        .get("property_bag")
        .and_then(|bag| bag.get("wait_duration_scaler"))
        .and_then(Value::as_f64)
        .unwrap_or(1.0) as f32;
    for node in graph
        .nodes
        .iter_mut()
        .filter(|node| node.kind == AnimKind::Wait)
    {
        node.duration *= wait_scale;
    }
    let anchor = anchor_to(control);
    Some(Arc::new(ControlAnims {
        key: key.to_owned(),
        graph,
        rest_alpha: control
            .properties
            .get("alpha")
            .and_then(Value::as_f64)
            .unwrap_or(1.0) as f32,
        rest_offset: rest_offset.map(|axis| axis as f32),
        rect: [rect.x, rect.y, rect.w, rect.h],
        anchor,
        born: context.born,
        clock: context.clock,
        disable_fast_forward: crate::widgets::bound_bool(control, "disable_anim_fast_forward")
            .unwrap_or(false),
        reset_name: control
            .properties
            .get("animation_reset_name")
            .and_then(Value::as_str)
            .filter(|name| !name.is_empty())
            .map(str::to_owned),
        has_sprite: control.control_type.as_deref() == Some("image"),
    }))
}

/// The static sprite uv and clip a draw's own animations replace.
pub(super) fn sprite_rest(control: &ResolvedControl, node: &mut NodeAnim) {
    let pair = |key: &str| {
        let items = control.properties.get(key)?.as_array()?;
        Some([
            items.first()?.as_f64()? as f32,
            items.get(1)?.as_f64()? as f32,
        ])
    };
    node.uv_rest = pair("uv").unwrap_or([0.0; 2]);
    node.uv_size_rest = pair("uv_size");
    if node
        .own
        .as_ref()
        .is_some_and(|own| own.writes(AnimKind::Clip))
    {
        node.clip_direction = Some(
            control
                .properties
                .get("clip_direction")
                .and_then(Value::as_str)
                .unwrap_or("left")
                .to_owned(),
        );
        node.clip_rest = crate::widgets::bound_number(control, "clip_ratio").unwrap_or(0.0) as f32;
    }
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
