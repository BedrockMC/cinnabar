//! Two-pass layout of a resolved control tree into positioned virtual-pixel rects.
//!
//! Measure (bottom-up) supplies the content extents `%c`/`%cm`/`%sm`/`default` need;
//! place (top-down) resolves each control's size against its parent, then positions
//! it by `anchor_from`/`anchor_to`/`offset`, packing `stack_panel` children end to
//! end with `fill` absorbing the leftover main-axis space. Everything is in the
//! virtual coordinate space of `root_size`; the virtual-to-physical scale is applied
//! downstream by the renderer and is deliberately not modelled here.
//!
//! `grid` and `scroll_view` are laid out as plain panels for now; their collection
//! and viewport behaviour arrives with data-binding in a later tranche.

use serde_json::Value;

use crate::expr::{self, AxisContext, Length, Resolved};
use crate::sidecar::TextureMeta;
use crate::tree::ResolvedControl;

/// A virtual-pixel rectangle, top-left origin.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

impl Rect {
    pub fn new(x: f64, y: f64, w: f64, h: f64) -> Self {
        Self { x, y, w, h }
    }

    /// The overlap of two rects, clamped so width/height never go negative.
    pub fn intersect(self, other: Rect) -> Rect {
        let x0 = self.x.max(other.x);
        let y0 = self.y.max(other.y);
        let x1 = (self.x + self.w).min(other.x + other.w);
        let y1 = (self.y + self.h).min(other.y + other.h);
        Rect::new(x0, y0, (x1 - x0).max(0.0), (y1 - y0).max(0.0))
    }
}

/// Natural size of a label's text, in virtual pixels. The real font binds later; a
/// caller without one can return zero, which lays a label out as an empty extent.
pub trait TextMeasure {
    fn extent(&self, text: &str) -> [f64; 2];
}

/// Resolves a `texture` path to its sidecar metadata (base size, nine-slice). The
/// atlas is bound later; only the metadata is needed to size and slice a sprite.
pub trait TextureSource {
    fn texture(&self, path: &str) -> Option<TextureMeta>;
}

/// The measurement backends layout and emit share.
pub struct LayoutEnv<'a> {
    pub text: &'a dyn TextMeasure,
    pub textures: &'a dyn TextureSource,
}

/// A placed control: the borrowed definition plus its resolved geometry and the
/// clip region it draws within.
#[derive(Clone, Debug)]
pub struct LaidOut<'a> {
    pub control: &'a ResolvedControl,
    pub rect: Rect,
    pub clip: Rect,
    pub layer: i32,
    pub alpha: f32,
    pub visible: bool,
    pub children: Vec<LaidOut<'a>>,
}

#[derive(Clone, Copy, PartialEq)]
enum Axis {
    X,
    Y,
}

/// Lay `root` out inside a virtual screen of `root_size`, positioning it as the lone
/// child of that screen.
pub fn layout<'a>(root: &'a ResolvedControl, root_size: [f64; 2], env: &LayoutEnv) -> LaidOut<'a> {
    let screen = Rect::new(0.0, 0.0, root_size[0], root_size[1]);
    let own = resolve_size(root, screen, intrinsic(root, env), env);
    let rect = place_by_anchor(root, screen, own, env);
    place_subtree(root, rect, screen, env)
}

fn place_subtree<'a>(
    control: &'a ResolvedControl,
    rect: Rect,
    parent_clip: Rect,
    env: &LayoutEnv,
) -> LaidOut<'a> {
    let child_clip = if clip_children(control) {
        parent_clip.intersect(rect)
    } else {
        parent_clip
    };
    let children = layout_children(control, rect, env)
        .into_iter()
        .map(|(child, child_rect)| place_subtree(child, child_rect, child_clip, env))
        .collect();
    LaidOut {
        control,
        rect,
        clip: parent_clip,
        layer: layer(control),
        alpha: alpha(control),
        visible: visible(control),
        children,
    }
}

/// Resolve the rects of `parent`'s direct children within `parent_rect`.
fn layout_children<'a>(
    parent: &'a ResolvedControl,
    parent_rect: Rect,
    env: &LayoutEnv,
) -> Vec<(&'a ResolvedControl, Rect)> {
    let sibling_max = siblings_max(parent, env);
    match stack_axis(parent) {
        Some(axis) => stack_children(parent, parent_rect, axis, sibling_max, env),
        None => parent
            .children
            .iter()
            .map(|child| {
                let own = resolve_size(child, parent_rect, sibling_max, env);
                (child, place_by_anchor(child, parent_rect, own, env))
            })
            .collect(),
    }
}

fn stack_children<'a>(
    parent: &'a ResolvedControl,
    parent_rect: Rect,
    axis: Axis,
    sibling_max: [f64; 2],
    env: &LayoutEnv,
) -> Vec<(&'a ResolvedControl, Rect)> {
    let main = axis;
    let cross = other(axis);
    let parent_main = axis_of(parent_rect, main);
    let parent_cross = axis_of(parent_rect, cross);

    // First resolve every child's cross size and its main size (fill deferred).
    let mut cross_sizes = Vec::with_capacity(parent.children.len());
    let mut main_sizes = Vec::with_capacity(parent.children.len());
    let mut fixed_total = 0.0;
    let mut fill_count = 0usize;
    for child in &parent.children {
        let content = content_extent(child, env);
        let child_max = children_max(child, env);
        let nat = natural(child, env);
        let cross_ctx = axis_context(
            parent_cross,
            None,
            content,
            child_max,
            sibling_max,
            nat,
            cross,
        );
        let cross_size = pixels_or(length(child, cross).eval(&cross_ctx), parent_cross);
        let main_ctx = axis_context(
            parent_main,
            Some((cross, cross_size)),
            content,
            child_max,
            sibling_max,
            nat,
            main,
        );
        match length(child, main).eval(&main_ctx) {
            Resolved::Pixels(value) => {
                fixed_total += value;
                main_sizes.push(Some(value));
            }
            Resolved::Fill => {
                fill_count += 1;
                main_sizes.push(None);
            }
        }
        cross_sizes.push(cross_size);
    }

    let leftover = (parent_main - fixed_total).max(0.0);
    let fill_each = if fill_count > 0 {
        leftover / fill_count as f64
    } else {
        0.0
    };

    let mut cursor = axis_min(parent_rect, main);
    let mut placed = Vec::with_capacity(parent.children.len());
    for (index, child) in parent.children.iter().enumerate() {
        let main_size = main_sizes[index].unwrap_or(fill_each);
        let cross_size = cross_sizes[index];
        let off = offset(child, parent_rect, env);
        let main_pos = cursor + axis_pick(off, main);
        let cross_pos = axis_min(parent_rect, cross)
            + parent_cross * anchor_frac(anchor_to(child), cross)
            - cross_size * anchor_frac(anchor_from(child), cross)
            + axis_pick(off, cross);
        placed.push((
            child,
            from_axes(main, main_pos, main_size, cross_pos, cross_size),
        ));
        cursor += main_size;
    }
    placed
}

/// The child's rect from its resolved size and anchor/offset within `parent_rect`.
fn place_by_anchor(
    control: &ResolvedControl,
    parent_rect: Rect,
    size: [f64; 2],
    env: &LayoutEnv,
) -> Rect {
    let from = anchor_from(control);
    let to = anchor_to(control);
    let off = offset(control, parent_rect, env);
    let x = parent_rect.x + parent_rect.w * anchor_frac(to, Axis::X)
        - size[0] * anchor_frac(from, Axis::X)
        + off[0];
    let y = parent_rect.y + parent_rect.h * anchor_frac(to, Axis::Y)
        - size[1] * anchor_frac(from, Axis::Y)
        + off[1];
    Rect::new(x, y, size[0], size[1])
}

/// Resolve a non-stack child's `[w, h]` against its parent, clamped by min/max.
fn resolve_size(
    control: &ResolvedControl,
    parent_rect: Rect,
    sibling_max: [f64; 2],
    env: &LayoutEnv,
) -> [f64; 2] {
    let content = content_extent(control, env);
    let child_max = children_max(control, env);
    let nat = natural(control, env);
    let width_ctx = axis_context(
        parent_rect.w,
        None,
        content,
        child_max,
        sibling_max,
        nat,
        Axis::X,
    );
    let width = pixels_or(length(control, Axis::X).eval(&width_ctx), parent_rect.w);
    let height_ctx = axis_context(
        parent_rect.h,
        Some((Axis::X, width)),
        content,
        child_max,
        sibling_max,
        nat,
        Axis::Y,
    );
    let height = pixels_or(length(control, Axis::Y).eval(&height_ctx), parent_rect.h);
    clamp_bounds(control, parent_rect, [width, height], content, nat, env)
}

fn clamp_bounds(
    control: &ResolvedControl,
    parent_rect: Rect,
    size: [f64; 2],
    content: [f64; 2],
    nat: Option<[f64; 2]>,
    _env: &LayoutEnv,
) -> [f64; 2] {
    let mut out = size;
    for (index, axis) in [Axis::X, Axis::Y].into_iter().enumerate() {
        let parent = axis_of(parent_rect, axis);
        let ctx = axis_context(parent, None, content, content, content, nat, axis);
        if let Some(max) = bound_length(control, "max_size", index) {
            out[index] = out[index].min(max.eval_pixels(&ctx));
        }
        if let Some(min) = bound_length(control, "min_size", index) {
            out[index] = out[index].max(min.eval_pixels(&ctx));
        }
    }
    out
}

/// Intrinsic (parent-independent) size, used when a parent aggregates this child for
/// its own `%c`/`%cm`. Parent-relative units resolve to zero here by design.
fn intrinsic(control: &ResolvedControl, env: &LayoutEnv) -> [f64; 2] {
    let content = content_extent(control, env);
    let child_max = children_max(control, env);
    let nat = natural(control, env);
    let width_ctx = axis_context(0.0, None, content, child_max, [0.0; 2], nat, Axis::X);
    let width = pixels_or(length(control, Axis::X).eval(&width_ctx), 0.0);
    let height_ctx = axis_context(
        0.0,
        Some((Axis::X, width)),
        content,
        child_max,
        [0.0; 2],
        nat,
        Axis::Y,
    );
    let height = pixels_or(length(control, Axis::Y).eval(&height_ctx), 0.0);
    [width, height]
}

/// The extent of a control's children, the value `%c` reports. A stack sums along
/// its main axis and takes the max across; other controls take the bounding max.
fn content_extent(control: &ResolvedControl, env: &LayoutEnv) -> [f64; 2] {
    if control.children.is_empty() {
        return [0.0, 0.0];
    }
    let sizes: Vec<[f64; 2]> = control
        .children
        .iter()
        .map(|child| intrinsic(child, env))
        .collect();
    match stack_axis(control) {
        Some(Axis::X) => [
            sizes.iter().map(|s| s[0]).sum(),
            sizes.iter().map(|s| s[1]).fold(0.0, f64::max),
        ],
        Some(Axis::Y) => [
            sizes.iter().map(|s| s[0]).fold(0.0, f64::max),
            sizes.iter().map(|s| s[1]).sum(),
        ],
        None => [
            sizes.iter().map(|s| s[0]).fold(0.0, f64::max),
            sizes.iter().map(|s| s[1]).fold(0.0, f64::max),
        ],
    }
}

/// Per-axis largest child, the value `%cm` reports.
fn children_max(control: &ResolvedControl, env: &LayoutEnv) -> [f64; 2] {
    control
        .children
        .iter()
        .map(|child| intrinsic(child, env))
        .fold([0.0, 0.0], |acc, size| {
            [acc[0].max(size[0]), acc[1].max(size[1])]
        })
}

/// Per-axis largest of a parent's children, the value `%sm` reports to each sibling.
fn siblings_max(parent: &ResolvedControl, env: &LayoutEnv) -> [f64; 2] {
    children_max(parent, env)
}

/// Natural content size: an image's texture `base_size`, a label's text extent.
fn natural(control: &ResolvedControl, env: &LayoutEnv) -> Option<[f64; 2]> {
    match control.control_type.as_deref() {
        Some("image") => texture_path(control)
            .and_then(|path| env.textures.texture(&path).map(|meta| meta.base_size)),
        Some("label") => Some(env.text.extent(&label_text(control))),
        _ => None,
    }
}

#[allow(clippy::too_many_arguments)]
fn axis_context(
    parent: f64,
    other_axis: Option<(Axis, f64)>,
    content: [f64; 2],
    child_max: [f64; 2],
    sibling_max: [f64; 2],
    natural: Option<[f64; 2]>,
    axis: Axis,
) -> AxisContext {
    let index = axis_index(axis);
    let (own_width, own_height) = match other_axis {
        Some((Axis::X, value)) => (Some(value), None),
        Some((Axis::Y, value)) => (None, Some(value)),
        None => (None, None),
    };
    AxisContext {
        parent,
        own_width,
        own_height,
        children: Some(content[index]),
        children_max: Some(child_max[index]),
        sibling_max: Some(sibling_max[index]),
        natural: natural.map(|n| n[index]),
    }
}

// --- property readers -------------------------------------------------------

fn length(control: &ResolvedControl, axis: Axis) -> Length {
    let index = axis_index(axis);
    match control.properties.get("size") {
        Some(Value::Array(items)) if items.len() >= 2 => {
            expr::length_from_value(&items[index]).unwrap_or_else(|_| Length::percent(100.0))
        }
        Some(scalar @ (Value::String(_) | Value::Number(_))) => {
            expr::length_from_value(scalar).unwrap_or_else(|_| Length::percent(100.0))
        }
        // Omitted size fills the parent; see the module note on `default`.
        _ => Length::percent(100.0),
    }
}

fn bound_length(control: &ResolvedControl, key: &str, index: usize) -> Option<Length> {
    match control.properties.get(key)? {
        Value::Array(items) if items.len() >= 2 => expr::length_from_value(&items[index]).ok(),
        scalar @ (Value::String(_) | Value::Number(_)) => expr::length_from_value(scalar).ok(),
        _ => None,
    }
}

fn offset(control: &ResolvedControl, parent_rect: Rect, _env: &LayoutEnv) -> [f64; 2] {
    let Some(Value::Array(items)) = control.properties.get("offset") else {
        return [0.0, 0.0];
    };
    if items.len() < 2 {
        return [0.0, 0.0];
    }
    let axis_value = |index: usize, axis: Axis| {
        let parent = axis_of(parent_rect, axis);
        let ctx = AxisContext {
            parent,
            ..AxisContext::default()
        };
        expr::length_from_value(&items[index])
            .map(|len| pixels_or(len.eval(&ctx), 0.0))
            .unwrap_or(0.0)
    };
    [axis_value(0, Axis::X), axis_value(1, Axis::Y)]
}

fn anchor_from(control: &ResolvedControl) -> [f64; 2] {
    anchor(control, "anchor_from")
}

fn anchor_to(control: &ResolvedControl) -> [f64; 2] {
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

fn anchor_frac(point: [f64; 2], axis: Axis) -> f64 {
    point[axis_index(axis)]
}

fn stack_axis(control: &ResolvedControl) -> Option<Axis> {
    if control.control_type.as_deref() != Some("stack_panel") {
        return None;
    }
    match control
        .properties
        .get("orientation")
        .and_then(Value::as_str)
    {
        Some("horizontal") => Some(Axis::X),
        // vertical is the JSON-UI default orientation.
        _ => Some(Axis::Y),
    }
}

fn clip_children(control: &ResolvedControl) -> bool {
    matches!(
        control.properties.get("clip_children"),
        Some(Value::Bool(true))
    )
}

fn layer(control: &ResolvedControl) -> i32 {
    control
        .properties
        .get("layer")
        .and_then(Value::as_i64)
        .map(|value| value as i32)
        .unwrap_or(0)
}

fn alpha(control: &ResolvedControl) -> f32 {
    control
        .properties
        .get("alpha")
        .and_then(Value::as_f64)
        .map(|value| value as f32)
        .unwrap_or(1.0)
}

/// `visible` honours a literal bool or `"true"`/`"false"`; an undecidable binding
/// stays visible, matching the lenient-remote-data rule.
fn visible(control: &ResolvedControl) -> bool {
    match control.properties.get("visible") {
        Some(Value::Bool(flag)) => *flag,
        Some(Value::String(text)) => text != "false",
        _ => true,
    }
}

fn texture_path(control: &ResolvedControl) -> Option<String> {
    control
        .properties
        .get("texture")
        .and_then(Value::as_str)
        .map(str::to_owned)
}

fn label_text(control: &ResolvedControl) -> String {
    match control.properties.get("text").and_then(Value::as_str) {
        // A `#binding` has no literal extent until data binds in a later tranche.
        Some(text) if !text.starts_with('#') => text.to_owned(),
        _ => String::new(),
    }
}

// --- axis helpers -----------------------------------------------------------

fn other(axis: Axis) -> Axis {
    match axis {
        Axis::X => Axis::Y,
        Axis::Y => Axis::X,
    }
}

fn axis_index(axis: Axis) -> usize {
    match axis {
        Axis::X => 0,
        Axis::Y => 1,
    }
}

fn axis_of(rect: Rect, axis: Axis) -> f64 {
    match axis {
        Axis::X => rect.w,
        Axis::Y => rect.h,
    }
}

fn axis_min(rect: Rect, axis: Axis) -> f64 {
    match axis {
        Axis::X => rect.x,
        Axis::Y => rect.y,
    }
}

fn axis_pick(pair: [f64; 2], axis: Axis) -> f64 {
    pair[axis_index(axis)]
}

fn from_axes(main: Axis, main_pos: f64, main_size: f64, cross_pos: f64, cross_size: f64) -> Rect {
    match main {
        Axis::X => Rect::new(main_pos, cross_pos, main_size, cross_size),
        Axis::Y => Rect::new(cross_pos, main_pos, cross_size, main_size),
    }
}

fn pixels_or(resolved: Resolved, fill: f64) -> f64 {
    match resolved {
        Resolved::Pixels(value) => value,
        Resolved::Fill => fill,
    }
}
