//! Two-pass layout of a resolved control tree into positioned virtual-pixel rects,
//! solving the same rules as the client's layout variables.
//!
//! Measure (bottom-up) supplies the content extents `%c`/`%cm`/`%sm`/`default` need;
//! place (top-down) resolves each control's size against its parent ([`size`]), then
//! positions it by `anchor_from`/`anchor_to`/`offset`, or as a [`stack`] item or
//! [`grid`] cell. Everything is in the virtual coordinate space of `root_size`; the
//! virtual-to-physical scale is applied downstream by the renderer.
//!
//! Layers are relative: a control draws at its parent's layer plus its own. Each
//! placed control carries a stable key (see [`crate::state`]) so the caller's
//! hover/press/scroll state can drive the engine-owned widget behaviour in
//! [`crate::widgets`].

use serde_json::Value;

use crate::anim::{Fade, Inherited, Motions};
use crate::sidecar::TextureMeta;
use crate::state::{LayoutReport, ViewState};
use crate::tree::ResolvedControl;
use crate::widgets::{self, ScrollFrame};

mod grid;
mod measure;
mod place;
mod size;
mod stack;

pub(crate) use grid::TEMPLATE_KEY as GRID_TEMPLATE_KEY;
pub use measure::MeasureCache;
pub(crate) use size::{font_scale, localizes};

use place::{motion, place_by_anchor};

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
    let own = size::resolve_size(root, [Some(screen.w), Some(screen.h)], [0.0; 2], env);
    let rect = place_by_anchor(root, screen, own, [0.0; 2], env);
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
        (0, true, false),
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
    (parent_layer, shown, packed): (i32, bool, bool),
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
    // A stack item or grid cell has no offset term, so its offset animation is inert.
    if !packed {
        motions.own.extend(motion(
            control,
            parent_rect,
            [rect.w, rect.h],
            &inherit,
            ctx.env,
        ));
    }
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
    let priority = stack::hidden_by_priority(control, rect, ctx.env);
    let packs = stack::orientation(control).is_some() || grid::is_grid(control);
    let mut children = Vec::with_capacity(placed.len());
    for (child, mut child_rect) in placed {
        let mut child_shown = !hidden.contains(&child.name)
            && !priority
                .get(measure::child_index(control, child))
                .copied()
                .unwrap_or(false);
        let mut clip_for_child = child_clip;
        // A dropdown's content lays out inside its named area, not its parent.
        if let Some((area, content)) = &dropdown
            && *content == child.name
            && let Some((_, area_rect, area_clip)) =
                ctx.ancestors.iter().rev().find(|(name, _, _)| name == area)
        {
            let area = [Some(area_rect.w), Some(area_rect.h)];
            let size = size::resolve_size(child, area, [0.0; 2], ctx.env);
            child_rect = place_by_anchor(child, *area_rect, size, [0.0; 2], ctx.env);
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
            (absolute_layer, child_shown, packs),
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
    if grid::is_grid(parent) {
        return grid::grid_children(parent, parent_rect, env);
    }
    if stack::orientation(parent).is_some() {
        return stack::stack_children(parent, parent_rect, env);
    }
    let extent = [Some(parent_rect.w), Some(parent_rect.h)];
    let sizes = measure::sizes(parent, extent, env);
    let siblings = measure::sibling_maxima(parent, &sizes);
    parent
        .children
        .iter()
        .zip(sizes)
        .map(|(child, own)| {
            (
                child,
                place_by_anchor(child, parent_rect, own, siblings, env),
            )
        })
        .collect()
}

// --- property readers -------------------------------------------------------

/// The axis a stack panel packs along; `none` and other controls have none.
fn stack_axis(control: &ResolvedControl) -> Option<Axis> {
    stack::main_axis(control)
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
    match control.properties.get("visible") {
        Some(Value::Bool(flag)) => *flag,
        Some(Value::String(text)) => text != "false",
        _ => true,
    }
}

fn visible(control: &ResolvedControl) -> bool {
    own_visible(control)
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
