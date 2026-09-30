//! Two-pass layout of a resolved control tree into positioned virtual-pixel rects.
//!
//! Measure (bottom-up) supplies the content extents `%c`/`%cm`/`%sm`/`default` need;
//! place (top-down) resolves each control's size against its parent, then positions
//! it by `anchor_from`/`anchor_to`/`offset`, packing `stack_panel` children end to
//! end with `fill` absorbing the leftover main-axis space. Everything is in the
//! virtual coordinate space of `root_size`; the virtual-to-physical scale is applied
//! downstream by the renderer and is deliberately not modelled here.
//!
//! Layers are relative: a control draws at its parent's layer plus its own. Each
//! placed control carries a stable key (see [`crate::state`]) so the caller's
//! hover/press/scroll state can drive the engine-owned widget behaviour in
//! [`crate::widgets`]. `grid` lays out as a plain panel; its cells arrive
//! pre-positioned from the binder.

use serde_json::Value;

use crate::anim::{Fade, Inherited, Motions};
use crate::expr::{self, AxisContext, Length, Resolved};
use crate::sidecar::TextureMeta;
use crate::state::{LayoutReport, ViewState};
use crate::tree::ResolvedControl;
use crate::widgets::{self, ScrollFrame};

mod grid;
mod measure;
mod place;

pub use measure::MeasureCache;

use grid::{fitted_columns, grid_children, grid_columns};
use place::{anchor_frac, anchor_from, anchor_to, motion, offset, place_by_anchor};

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

    /// The extent when wrapped at `max_width`; a measurer without wrapping keeps
    /// the single-line extent.
    fn wrapped(&self, text: &str, max_width: f64) -> [f64; 2] {
        let _ = max_width;
        self.extent(text)
    }

    /// A localizing label's text as it will draw; measurers without a language
    /// table measure it as written.
    fn localize<'a>(&self, text: &'a str) -> std::borrow::Cow<'a, str> {
        std::borrow::Cow::Borrowed(text)
    }
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
    /// Stable address for [`ViewState`] lookups.
    pub key: String,
    pub rect: Rect,
    pub clip: Rect,
    /// Absolute draw layer (the parent's plus this control's own).
    pub layer: i32,
    pub alpha: f32,
    /// Animations scaling `alpha` at paint time, own and propagated.
    pub fades: Vec<Fade>,
    /// Offset animations displacing this control and its clip at paint time.
    pub motions: Motions,
    pub visible: bool,
    /// Fraction clipped off a progress image by its widget (`clip_direction`).
    pub clip_ratio: Option<f32>,
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
    layout_with(root, root_size, env, &ViewState::default()).0
}

/// [`layout`] driven by live interaction state, also reporting scroll extents.
pub fn layout_with<'a>(
    root: &'a ResolvedControl,
    root_size: [f64; 2],
    env: &LayoutEnv,
    state: &ViewState,
) -> (LaidOut<'a>, LayoutReport) {
    lay_out(root, root_size, env, state, false)
}

/// [`layout_with`] over `cache`'s measurements that omits hidden controls'
/// subtrees and scroll content wholly outside its viewport, so a long list costs
/// only what it shows.
pub(crate) fn layout_culled<'a>(
    root: &'a ResolvedControl,
    root_size: [f64; 2],
    env: &LayoutEnv,
    state: &ViewState,
    cache: &mut MeasureCache,
) -> (LaidOut<'a>, LayoutReport) {
    cache.enter(root);
    let laid = lay_out(root, root_size, env, state, true);
    cache.leave();
    laid
}

/// A culling layout keeps the memos its caller entered.
fn lay_out<'a>(
    root: &'a ResolvedControl,
    root_size: [f64; 2],
    env: &LayoutEnv,
    state: &ViewState,
    cull: bool,
) -> (LaidOut<'a>, LayoutReport) {
    if !cull {
        measure::reset();
    }
    let screen = Rect::new(0.0, 0.0, root_size[0], root_size[1]);
    let own = resolve_size(root, screen, measure::intrinsic(root, env, [None; 2]), env);
    let rect = place_by_anchor(root, screen, own, env);
    let mut ctx = PlaceCtx {
        env,
        state,
        cull,
        report: LayoutReport::default(),
        scrolls: Vec::new(),
        sliders: Vec::new(),
        ancestors: Vec::new(),
    };
    let key = child_key("", root);
    let laid = place_subtree(
        root,
        key,
        rect,
        screen,
        (0, true),
        &Inherited::default(),
        &mut ctx,
    );
    (laid, ctx.report)
}

struct PlaceCtx<'e, 'x> {
    env: &'e LayoutEnv<'x>,
    state: &'e ViewState,
    /// Skip placing scroll content wholly outside its viewport.
    cull: bool,
    report: LayoutReport,
    scrolls: Vec<ScrollFrame>,
    /// Enclosing sliders: their fraction plus progress child names.
    sliders: Vec<(f64, [Option<String>; 3])>,
    /// Enclosing controls' names, rects, and child clips, for `dropdown_area`.
    ancestors: Vec<(String, Rect, Rect)>,
}

/// `parent/name`, with `[index]` on factory instances so repeated names stay unique.
fn child_key(parent: &str, control: &ResolvedControl) -> String {
    let mut key = String::with_capacity(parent.len() + control.name.len() + 6);
    key.push_str(parent);
    key.push('/');
    key.push_str(&control.name);
    if let Some(index) = control
        .properties
        .get("collection_index")
        .and_then(Value::as_u64)
    {
        key.push('[');
        key.push_str(&index.to_string());
        key.push(']');
    }
    key
}

#[allow(clippy::too_many_arguments)]
fn place_subtree<'a>(
    control: &'a ResolvedControl,
    key: String,
    rect: Rect,
    parent_clip: Rect,
    (parent_layer, shown): (i32, bool),
    inherited: &Inherited,
    ctx: &mut PlaceCtx,
) -> LaidOut<'a> {
    let (own_alpha, fades, mut inherit) = inherited.apply(control, alpha(control));
    let clips = clip_children(control);
    let child_clip = if clips {
        parent_clip.intersect(rect)
    } else {
        parent_clip
    };
    let parent_rect = ctx
        .ancestors
        .last()
        .map_or(parent_clip, |(_, parent, _)| *parent);
    let mut motions = inherited.motions.clone();
    motions
        .own
        .extend(motion(control, parent_rect, [rect.w, rect.h], &inherit));
    inherit.motions = Motions {
        clip: if clips {
            motions.own.clone()
        } else {
            motions.clip.clone()
        },
        own: motions.own.clone(),
    };
    let absolute_layer = parent_layer.saturating_add(layer(control));
    let scroll = ScrollFrame::open(control, &key, ctx.state);
    let opened_scroll = scroll.is_some();
    if let Some(frame) = scroll {
        ctx.scrolls.push(frame);
    }
    let slider = widgets::slider_fraction(control).map(|f| (f, widgets::slider_names(control)));
    let opened_slider = slider.is_some();
    if let Some(entry) = slider {
        ctx.sliders.push(entry);
    }
    let hidden = widgets::hidden_state_children(control, &key, ctx.state);
    let dropdown = widgets::dropdown_area(control);
    ctx.ancestors.push((control.name.clone(), rect, child_clip));
    // A culling layout leaves a hidden control's subtree unplaced: nothing in it draws.
    let placed = if ctx.cull && !visible(control) {
        Vec::new()
    } else {
        measure::placed_children(control, rect, ctx.env)
    };
    let mut children = Vec::with_capacity(placed.len());
    for (child, mut child_rect) in placed {
        let mut child_shown = !hidden.contains(&child.name);
        let mut clip_for_child = child_clip;
        // A dropdown's content lays out inside its named area, not its parent.
        if let Some((area, content)) = &dropdown
            && *content == child.name
            && let Some((_, area_rect, area_clip)) =
                ctx.ancestors.iter().rev().find(|(name, _, _)| name == area)
        {
            let size = resolve_size(child, *area_rect, siblings_max(control, ctx.env), ctx.env);
            child_rect = place_by_anchor(child, *area_rect, size, ctx.env);
            clip_for_child = *area_clip;
        }
        if let Some(frame) = ctx.scrolls.last_mut() {
            if frame.metrics.is_none() && child.name == frame.content {
                child_rect = frame.place_content(rect, child_rect);
            } else if child.name == frame.bar_box
                && child.control_type.as_deref() == Some("scrollbar_box")
            {
                match frame.place_box(rect, child_rect) {
                    Some(placed) => child_rect = placed,
                    None => child_shown = false,
                }
            }
        }
        if opened_slider
            && let Some((fraction, names)) = ctx.sliders.last()
            && names[0].as_deref() == Some(child.name.as_str())
        {
            child_rect = widgets::slider_box_rect(rect, child_rect, *fraction);
        }
        // Scroll content wholly outside its viewport neither draws nor takes input.
        if ctx.cull
            && ctx
                .scrolls
                .last()
                .is_some_and(|frame| frame.metrics.is_some())
            && disjoint(child_rect, clip_for_child)
        {
            continue;
        }
        let next_key = child_key(&key, child);
        children.push(place_subtree(
            child,
            next_key,
            child_rect,
            clip_for_child,
            (absolute_layer, child_shown),
            &inherit,
            ctx,
        ));
    }
    ctx.ancestors.pop();
    if opened_slider {
        ctx.sliders.pop();
    }
    if opened_scroll
        && let Some(frame) = ctx.scrolls.pop()
        && let Some(metrics) = frame.metrics
    {
        ctx.report.scrolls.insert(frame.key, metrics);
    }
    LaidOut {
        control,
        clip_ratio: progress_clip(control, &ctx.sliders),
        key,
        rect,
        clip: parent_clip,
        layer: absolute_layer,
        alpha: own_alpha,
        fades,
        motions,
        visible: shown && visible(control),
        children,
    }
}

/// True when `rect` and `clip` share no area.
fn disjoint(rect: Rect, clip: Rect) -> bool {
    rect.x >= clip.x + clip.w
        || rect.y >= clip.y + clip.h
        || rect.x + rect.w <= clip.x
        || rect.y + rect.h <= clip.y
}

/// A bound `clip_ratio`, or a slider progress image revealing its fraction.
fn progress_clip(control: &ResolvedControl, sliders: &[(f64, [Option<String>; 3])]) -> Option<f32> {
    if let Some((fraction, names)) = sliders.last()
        && (names[1].as_deref() == Some(control.name.as_str())
            || names[2].as_deref() == Some(control.name.as_str()))
    {
        return Some((1.0 - fraction) as f32);
    }
    widgets::bound_number(control, "clip_ratio").map(|ratio| ratio.clamp(0.0, 1.0) as f32)
}

/// Resolve the rects of `parent`'s direct children within `parent_rect`.
fn layout_children<'a>(
    parent: &'a ResolvedControl,
    parent_rect: Rect,
    env: &LayoutEnv,
) -> Vec<(&'a ResolvedControl, Rect)> {
    let sibling_max = siblings_max(parent, env);
    if let Some(columns) = grid_columns(parent) {
        return grid_children(parent, parent_rect, columns, sibling_max, env);
    }
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
        let content = content_extent(child, env, [None; 2]);
        let nat = natural(child, env, None);
        let cross_of = |main_size: Option<f64>| {
            let other = main_size.map(|size| (main, size));
            let own = known_size(other);
            let content = content_extent(child, env, own);
            let cross_ctx = axis_context(
                parent_cross,
                other,
                content,
                children_max(child, env, own),
                sibling_max,
                nat,
                cross,
            );
            let mut cross_size = pixels_or(eval_length(child, cross, &cross_ctx), parent_cross);
            let inherit = match cross {
                Axis::X => "inherit_max_sibling_width",
                Axis::Y => "inherit_max_sibling_height",
            };
            if matches!(child.properties.get(inherit), Some(Value::Bool(true))) {
                cross_size = cross_size.max(axis_pick(sibling_max, cross));
            }
            clamp_axis(child, parent_rect, cross, cross_size, content, nat)
        };
        let main_of = |cross_size: Option<f64>| {
            // A child's cross size is known before its main size, so wrapped text
            // and `%c` content measure at it.
            let own = known_size(cross_size.map(|size| (cross, size)));
            let main_ctx = axis_context(
                parent_main,
                cross_size.map(|size| (cross, size)),
                content_extent(child, env, own),
                children_max(child, env, own),
                sibling_max,
                natural(child, env, own[0]),
                main,
            );
            // An invisible stack child collapses instead of holding its slot;
            // `max_size`/`min_size` bound a visible one (the start screen's
            // signing-in label wraps at 120px).
            if !visible(child) {
                return Resolved::Pixels(0.0);
            }
            match eval_length(child, main, &main_ctx) {
                Resolved::Pixels(value) => {
                    Resolved::Pixels(clamp_axis(child, parent_rect, main, value, content, nat))
                }
                Resolved::Fill => Resolved::Fill,
            }
        };
        // The cross size resolves first unless it depends on the main size: a
        // vertical stack child whose width follows its height, or a horizontal
        // one whose height reads its width (`[20, "100%x"]`).
        let main_first = match main {
            Axis::Y => height_first(child),
            Axis::X => {
                !height_first(child)
                    && axis_units(child, Axis::Y)
                        .is_some_and(|units| units.contains(&expr::Unit::PercentX))
            }
        };
        let (cross_size, resolved) = if main_first {
            let resolved = main_of(None);
            let main_size = match resolved {
                Resolved::Pixels(value) => Some(value),
                Resolved::Fill => None,
            };
            (cross_of(main_size), resolved)
        } else {
            let cross_size = cross_of(None);
            (cross_size, main_of(Some(cross_size)))
        };
        match resolved {
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
        let size = match main {
            Axis::X => [main_size, cross_size],
            Axis::Y => [cross_size, main_size],
        };
        let off = offset(child, parent_rect, size);
        let main_pos = cursor + axis_pick(off, main);
        let cross_pos = axis_min(parent_rect, cross)
            + parent_cross * anchor_frac(anchor_from(child), cross)
            - cross_size * anchor_frac(anchor_to(child), cross)
            + axis_pick(off, cross);
        placed.push((
            child,
            from_axes(main, main_pos, main_size, cross_pos, cross_size),
        ));
        cursor += main_size;
    }
    placed
}

/// Resolve a non-stack child's `[w, h]` against its parent, clamped by min/max.
fn resolve_size(
    control: &ResolvedControl,
    parent_rect: Rect,
    sibling_max: [f64; 2],
    env: &LayoutEnv,
) -> [f64; 2] {
    let axis = |axis: Axis, other: Option<(Axis, f64)>| {
        let parent = axis_of(parent_rect, axis);
        let own = known_size(other);
        let ctx = axis_context(
            parent,
            other,
            content_extent(control, env, own),
            children_max(control, env, own),
            sibling_max,
            natural(control, env, own[0]),
            axis,
        );
        pixels_or(eval_length(control, axis, &ctx), parent)
    };
    let [width, height] = in_dependency_order(control, axis);
    let own = if height_first(control) {
        [Some(width), Some(height)]
    } else {
        [Some(width), None]
    };
    let content = content_extent(control, env, own);
    let nat = natural(control, env, Some(width));
    let mut size = clamp_bounds(control, parent_rect, [width, height], content, nat, true);
    for (index, key) in ["inherit_max_sibling_width", "inherit_max_sibling_height"]
        .into_iter()
        .enumerate()
    {
        if matches!(control.properties.get(key), Some(Value::Bool(true))) {
            size[index] = size[index].max(sibling_max[index]);
        }
    }
    size
}

/// `[w, h]` from `resolve(axis, other_axis)`, width first unless [`height_first`].
fn in_dependency_order(
    control: &ResolvedControl,
    mut resolve: impl FnMut(Axis, Option<(Axis, f64)>) -> f64,
) -> [f64; 2] {
    if height_first(control) {
        let height = resolve(Axis::Y, None);
        [resolve(Axis::X, Some((Axis::Y, height))), height]
    } else {
        let width = resolve(Axis::X, None);
        [width, resolve(Axis::Y, Some((Axis::X, width)))]
    }
}

/// Whether the height resolves before the width: the width reads the height
/// (`["100%y", 32]`), or reads children that may, while the height reads neither.
fn height_first(control: &ResolvedControl) -> bool {
    use expr::Unit::{PercentChildren, PercentChildrenMax, PercentX, PercentY};
    let height_free = matches!(
        axis_units(control, Axis::Y),
        Some(units) if !units.iter().any(|unit| matches!(unit, PercentX | PercentChildren | PercentChildrenMax))
    );
    height_free
        && axis_units(control, Axis::X).is_some_and(|units| {
            units
                .iter()
                .any(|unit| matches!(unit, PercentY | PercentChildren | PercentChildrenMax))
        })
}

/// The units the size on `axis` sums, or `None` for `default`/`fill`.
fn axis_units(control: &ResolvedControl, axis: Axis) -> Option<Vec<expr::Unit>> {
    memo_length(
        control,
        axis_index(axis) as u8,
        || Some(length(control, axis)),
        |length| match length {
            Some(Length::Terms(terms)) => Some(terms.iter().map(|term| term.unit).collect()),
            _ => None,
        },
    )
}

/// The control's `[width, height]` as far as `other` carries it.
fn known_size(other: Option<(Axis, f64)>) -> [Option<f64>; 2] {
    match other {
        Some((Axis::X, width)) => [Some(width), None],
        Some((Axis::Y, height)) => [None, Some(height)],
        None => [None; 2],
    }
}

/// `size` on `axis` after the control's min/max bounds on that axis.
fn clamp_axis(
    control: &ResolvedControl,
    parent_rect: Rect,
    axis: Axis,
    size: f64,
    content: [f64; 2],
    nat: Option<[f64; 2]>,
) -> f64 {
    let mut both = [0.0; 2];
    both[axis_index(axis)] = size;
    let clamped = clamp_bounds(control, parent_rect, both, content, nat, true);
    clamped[axis_index(axis)]
}

/// Clamp by min/max; while the parent's size is still unknown (it sizes to its
/// children), a parent-relative bound does not constrain the child.
fn clamp_bounds(
    control: &ResolvedControl,
    parent_rect: Rect,
    size: [f64; 2],
    content: [f64; 2],
    nat: Option<[f64; 2]>,
    parent_known: bool,
) -> [f64; 2] {
    // A childless label's `%c` is its text: `max_size: ["100%c", 10]` fits the text.
    let content = match nat {
        Some(text) if control.children.is_empty() => text,
        _ => content,
    };
    let mut out = size;
    // A leaf's content is its own natural size (a label's `max_size: 100%c`).
    let content = nat
        .filter(|_| control.children.is_empty())
        .unwrap_or(content);
    for (index, axis) in [Axis::X, Axis::Y].into_iter().enumerate() {
        let parent = axis_of(parent_rect, axis);
        let bound = |key: &str, unknown: f64| {
            let parent = if parent_known { parent } else { unknown };
            let ctx = axis_context(parent, None, content, content, content, nat, axis);
            eval_bound(control, key, index, &ctx)
        };
        if let Some(max) = bound("max_size", f64::INFINITY) {
            out[index] = out[index].min(max);
        }
        if let Some(min) = bound("min_size", 0.0) {
            out[index] = out[index].max(min);
        }
    }
    out
}

fn memo_length<R>(
    control: &ResolvedControl,
    slot: u8,
    read: impl FnOnce() -> Option<Length>,
    eval: impl FnOnce(Option<&Length>) -> R,
) -> R {
    let key = (control as *const ResolvedControl as usize, slot);
    measure::LENGTHS.with(|memo| {
        let mut memo = memo.borrow_mut();
        let length = memo.entry(key).or_insert_with(read);
        eval(length.as_ref())
    })
}

fn eval_length(control: &ResolvedControl, axis: Axis, ctx: &AxisContext) -> Resolved {
    memo_length(
        control,
        axis_index(axis) as u8,
        || Some(length(control, axis)),
        |length| length.map_or(Resolved::Pixels(0.0), |length| length.eval(ctx)),
    )
}

fn eval_bound(
    control: &ResolvedControl,
    key: &str,
    index: usize,
    ctx: &AxisContext,
) -> Option<f64> {
    let slot = if key == "max_size" { 2 } else { 4 } + index as u8;
    memo_length(
        control,
        slot,
        || bound_length(control, key, index),
        |length| length.map(|length| length.eval_pixels(ctx)),
    )
}

/// The extent of a control's children, the value `%c` reports. A stack sums along
/// its main axis and takes the max across; other controls take the bounding max.
/// `own` is this control's resolved `[width, height]` where already known.
fn content_extent(control: &ResolvedControl, env: &LayoutEnv, own: [Option<f64>; 2]) -> [f64; 2] {
    measure::children(control, env, own).content
}

/// Per-axis largest child, the value `%cm` reports.
fn children_max(control: &ResolvedControl, env: &LayoutEnv, own: [Option<f64>; 2]) -> [f64; 2] {
    measure::children(control, env, own).maximum
}

/// Per-axis largest of a parent's children, the value `%sm` reports to each sibling.
fn siblings_max(parent: &ResolvedControl, env: &LayoutEnv) -> [f64; 2] {
    children_max(parent, env, [None; 2])
}

/// Natural content size: an opted-in image's texture `base_size`, a label's text extent
/// (wrapped at `width` when known), scaled by `font_scale_factor`.
fn natural(control: &ResolvedControl, env: &LayoutEnv, width: Option<f64>) -> Option<[f64; 2]> {
    if !matches!(control.control_type.as_deref(), Some("label" | "image")) {
        return None;
    }
    measure::natural(control, width, || natural_uncached(control, env, width))
}

/// Read a label or opted-in texture size on this layout's first request at `width`.
fn natural_uncached(
    control: &ResolvedControl,
    env: &LayoutEnv,
    width: Option<f64>,
) -> Option<[f64; 2]> {
    match control.control_type.as_deref() {
        // An image sizes to its texture only when it opts in; otherwise a
        // default axis fills the parent like any control.
        Some("image")
            if widgets::bound_bool(control, "default_size_scales_to_ratio") == Some(true) =>
        {
            texture_path(control)
                .and_then(|path| env.textures.texture(&path).map(|meta| meta.base_size))
        }
        Some("label") => {
            let scale = font_scale(control);
            let text = label_text(control);
            let text = if localizes(control) {
                env.text.localize(&text)
            } else {
                std::borrow::Cow::Borrowed(text.as_str())
            };
            let [w, h] = match width {
                Some(width) if width > 0.0 => env.text.wrapped(&text, width / scale),
                _ => env.text.extent(&text),
            };
            Some([w * scale, h * scale])
        }
        _ => None,
    }
}

/// A label's glyph scale: `font_scale_factor` (1 when absent or non-positive)
/// times its `font_size` step.
pub(crate) fn font_scale(control: &ResolvedControl) -> f64 {
    let factor = widgets::bound_number(control, "font_scale_factor")
        .filter(|scale| *scale > 0.0)
        .unwrap_or(1.0);
    factor * font_size_scale(control)
}

/// Glyph scale of a `font_size` (small/normal/large/extra_large); needs native
/// measurement of the client's font-size table.
fn font_size_scale(control: &ResolvedControl) -> f64 {
    match control.properties.get("font_size").and_then(Value::as_str) {
        Some("small") => 0.75,
        Some("large") => 1.5,
        Some("extra_large") => 2.0,
        _ => 1.0,
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

/// A control's size on `axis`. An omitted size is `default`: a label's text
/// size (an image's texture size with `default_size_scales_to_ratio`), a stack
/// panel's children along its axis, else the parent's full extent.
fn length(control: &ResolvedControl, axis: Axis) -> Length {
    let index = axis_index(axis);
    let explicit = match control.properties.get("size") {
        Some(Value::Array(items)) if items.len() >= 2 => {
            Some(expr::length_from_value(&items[index]).unwrap_or_else(|_| Length::percent(100.0)))
        }
        Some(scalar @ (Value::String(_) | Value::Number(_))) => {
            Some(expr::length_from_value(scalar).unwrap_or_else(|_| Length::percent(100.0)))
        }
        _ => None,
    };
    // A grid sizes to its cells, except one listing them with no size, which fills.
    let is_grid = control.control_type.as_deref() == Some("grid")
        && (explicit.is_some() || control.properties.contains_key("grid_item_template"));
    match explicit {
        Some(Length::Default) | None if stack_axis(control) == Some(axis) || is_grid => {
            expr::parse_length("100%c").unwrap_or(Length::Default)
        }
        Some(length) => length,
        None => Length::Default,
    }
}

fn bound_length(control: &ResolvedControl, key: &str, index: usize) -> Option<Length> {
    match control.properties.get(key)? {
        Value::Array(items) if items.len() >= 2 => expr::length_from_value(&items[index]).ok(),
        scalar @ (Value::String(_) | Value::Number(_)) => expr::length_from_value(scalar).ok(),
        _ => None,
    }
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
    ["clips_children", "clip_children"]
        .iter()
        .any(|key| matches!(control.properties.get(*key), Some(Value::Bool(true))))
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
pub(crate) fn own_visible(control: &ResolvedControl) -> bool {
    visible(control)
}

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

/// A label localizes its text unless `localize` is `false`.
pub(crate) fn localizes(control: &ResolvedControl) -> bool {
    control.properties.get("localize") != Some(&Value::Bool(false))
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
