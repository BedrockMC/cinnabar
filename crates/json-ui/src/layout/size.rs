//! Sizing: a control's `[w, h]` from its `size`, `min_size` and `max_size`
//! rules, solved in the order their own-axis references (`%x`, `%y`) need and
//! clamped before anything reads them, as the client's `LayoutVariable::satisfy`
//! does.

use serde_json::Value;

use super::{Axis, LayoutEnv, ResolvedControl, axis_index, measure, other};
use crate::expr::{self, AxisContext, Length, Resolved, Unit};
use crate::widgets;

/// The client's bounds when a control has no `min_size`/`max_size` rule.
const NO_MIN: f64 = -32768.0;
const NO_MAX: f64 = 32767.0;

/// Memo slots of the parsed lengths: size, then max, then min, per axis.
const SIZE_SLOT: u8 = 0;
const MAX_SLOT: u8 = 2;
const MIN_SLOT: u8 = 4;

/// Resolve `control`'s `[w, h]` under a parent whose axes may be unknown (a
/// parent sizing to its children); `siblings` is the largest sibling per axis,
/// which `%sm` reads.
pub(super) fn resolve_size(
    control: &ResolvedControl,
    parent: [Option<f64>; 2],
    siblings: [f64; 2],
    env: &LayoutEnv,
) -> [f64; 2] {
    let first = if height_first(control) {
        Axis::Y
    } else {
        Axis::X
    };
    let mut own = [None; 2];
    for axis in [first, other(first)] {
        let size = match axis_rule(control, axis, parent, own, siblings, env) {
            (Resolved::Pixels(value), ctx) => clamp(control, axis, value, &ctx, parent),
            // `fill` off a stack's main axis is an empty rule: zero, then bounds.
            (Resolved::Fill, ctx) => clamp(control, axis, 0.0, &ctx, parent),
        };
        own[axis_index(axis)] = Some(size);
    }
    [own[0].unwrap_or(0.0), own[1].unwrap_or(0.0)]
}

/// The size rule on `axis`, unclamped, with the context it evaluated in.
pub(super) fn axis_rule(
    control: &ResolvedControl,
    axis: Axis,
    parent: [Option<f64>; 2],
    own: [Option<f64>; 2],
    siblings: [f64; 2],
    env: &LayoutEnv,
) -> (Resolved, AxisContext) {
    let ctx = context(control, axis, parent, own, siblings, env);
    if let Some(cells) = super::grid::own_size(control, axis, own, env) {
        return (Resolved::Pixels(cells), ctx);
    }
    if let Some(animated) = animated(control, axis, &ctx) {
        return (Resolved::Pixels(animated), ctx);
    }
    let resolved = memo_length(
        control,
        SIZE_SLOT + axis_index(axis) as u8,
        || Some(length(control, axis)),
        |length| length.map_or(Resolved::Pixels(0.0), |length| length.eval(&ctx)),
    );
    (resolved, ctx)
}

/// A `size` animation's value on `axis` at the layout clock, its ends measured
/// like the size rule; `None` without an animation or a clock.
fn animated(control: &ResolvedControl, axis: Axis, ctx: &AxisContext) -> Option<f64> {
    let now = measure::clock()?;
    let slide: crate::anim::Slide =
        serde_json::from_value(control.properties.get(crate::anim::RESIZE_KEY)?.clone()).ok()?;
    let age = now - widgets::bound_number(control, crate::anim::BORN_KEY).unwrap_or(0.0);
    if slide.running(age) {
        measure::note_animating();
    }
    let index = axis_index(axis);
    let measure = |vector: &Value| match vector {
        Value::Array(items) => items
            .get(index)
            .map_or(Length::Default, expr::length_from_value)
            .eval_pixels(ctx),
        _ => length(control, axis).eval_pixels(ctx),
    };
    let rest = length(control, axis).eval_pixels(ctx);
    Some(slide.axis_at(age, rest, measure))
}

/// `value` within the control's bounds on `axis`: an over-large value takes the
/// maximum even when the minimum exceeds it. A parent-relative bound under an
/// unknown parent axis does not constrain.
pub(super) fn clamp(
    control: &ResolvedControl,
    axis: Axis,
    value: f64,
    ctx: &AxisContext,
    parent: [Option<f64>; 2],
) -> f64 {
    let index = axis_index(axis);
    let known = parent[index].is_some();
    let max = bound(control, MAX_SLOT, axis, ctx, known, f64::INFINITY).unwrap_or(NO_MAX);
    let min = bound(control, MIN_SLOT, axis, ctx, known, 0.0).unwrap_or(NO_MIN);
    if max < value {
        max
    } else if value <= min {
        min
    } else {
        value
    }
}

/// `value` under only the maximum on `axis` (a stack `fill` child's minimum is
/// forced to zero).
pub(super) fn clamp_max(
    control: &ResolvedControl,
    axis: Axis,
    value: f64,
    ctx: &AxisContext,
    parent: [Option<f64>; 2],
) -> f64 {
    let known = parent[axis_index(axis)].is_some();
    match bound(control, MAX_SLOT, axis, ctx, known, f64::INFINITY) {
        Some(max) if max < value => max,
        _ => value,
    }
}

/// A bound rule's pixels, or `None` without one (`default`, `fill`, no terms).
fn bound(
    control: &ResolvedControl,
    slot: u8,
    axis: Axis,
    ctx: &AxisContext,
    parent_known: bool,
    unknown_parent: f64,
) -> Option<f64> {
    let index = axis_index(axis);
    let key = if slot == MAX_SLOT {
        "max_size"
    } else {
        "min_size"
    };
    memo_length(
        control,
        slot + index as u8,
        || bound_length(control, key, index),
        |length| {
            length.map(|length| {
                if !parent_known && length.uses(Unit::Percent) {
                    let ctx = AxisContext {
                        parent: unknown_parent,
                        ..*ctx
                    };
                    return length.eval_pixels(&ctx);
                }
                length.eval_pixels(ctx)
            })
        },
    )
}

/// Whether a `max_size` rule exists on `axis`.
pub(super) fn has_max(control: &ResolvedControl, axis: Axis) -> bool {
    let index = axis_index(axis);
    memo_length(
        control,
        MAX_SLOT + index as u8,
        || bound_length(control, "max_size", index),
        |length| length.is_some(),
    )
}

/// What `axis`'s rules evaluate against. Own sizes feed content measurement
/// only where they resolved against a known parent axis.
fn context(
    control: &ResolvedControl,
    axis: Axis,
    parent: [Option<f64>; 2],
    own: [Option<f64>; 2],
    siblings: [f64; 2],
    env: &LayoutEnv,
) -> AxisContext {
    let index = axis_index(axis);
    let known = [
        own[0].filter(|_| parent[0].is_some()),
        own[1].filter(|_| parent[1].is_some()),
    ];
    let children = measure::children(control, env, known);
    AxisContext {
        parent: parent[index].unwrap_or(0.0),
        own_width: own[0],
        own_height: own[1],
        children: Some(children.content[index]),
        children_max: Some(children.maximum[index]),
        sibling_max: Some(siblings[index]),
        natural: natural(control, axis, own, known[0], env),
    }
}

/// Rule properties read on every solve, derived once per layout.
#[derive(Clone, Copy)]
pub(super) struct Flags {
    pub(super) height_first: bool,
    pub(super) reads_sibling_max: bool,
    /// `inherit_max_sibling_width`, `inherit_max_sibling_height`.
    pub(super) inherits: [bool; 2],
}

pub(super) fn flags(control: &ResolvedControl) -> Flags {
    let address = std::ptr::from_ref(control).addr();
    if let Some(flags) = measure::FLAGS.with(|memo| memo.borrow().get(&address).copied()) {
        return flags;
    }
    let flags = Flags {
        height_first: solves_height_first(control),
        reads_sibling_max: [Axis::X, Axis::Y]
            .into_iter()
            .any(|axis| reads(control, axis, &[Unit::PercentSiblingMax])),
        inherits: ["inherit_max_sibling_width", "inherit_max_sibling_height"]
            .map(|key| control.properties.get(key) == Some(&Value::Bool(true))),
    };
    measure::FLAGS.with(|memo| memo.borrow_mut().insert(address, flags));
    flags
}

/// Whether the height resolves before the width.
pub(super) fn height_first(control: &ResolvedControl) -> bool {
    flags(control).height_first
}

/// The width's rules read the height or the children while the height's read
/// neither, or a ratio-scaled image derives its default width from its height.
fn solves_height_first(control: &ResolvedControl) -> bool {
    use Unit::{PercentChildren, PercentChildrenMax, PercentX, PercentY};
    let width_reads = reads(
        control,
        Axis::X,
        &[PercentY, PercentChildren, PercentChildrenMax],
    );
    let height_free = with_size(control, Axis::Y, |length| {
        matches!(length, Some(Length::Terms(_)))
    }) && !reads(
        control,
        Axis::Y,
        &[PercentX, PercentChildren, PercentChildrenMax],
    );
    (width_reads && height_free) || ratio_scaled_width(control)
}

/// A ratio-scaled image whose width alone is `default` takes it from its height.
fn ratio_scaled_width(control: &ResolvedControl) -> bool {
    scales_to_ratio(control)
        && with_size(control, Axis::X, |length| {
            matches!(length, Some(Length::Default))
        })
        && !with_size(control, Axis::Y, |length| {
            matches!(length, Some(Length::Default))
        })
}

/// Whether `axis`'s size or bound rules use any of `units`.
pub(super) fn reads(control: &ResolvedControl, axis: Axis, units: &[Unit]) -> bool {
    let index = axis_index(axis) as u8;
    let uses = |length: Option<&Length>| {
        length.is_some_and(|length| units.iter().any(|unit| length.uses(*unit)))
    };
    memo_length(
        control,
        SIZE_SLOT + index,
        || Some(length(control, axis)),
        uses,
    ) || memo_length(
        control,
        MAX_SLOT + index,
        || bound_length(control, "max_size", index as usize),
        uses,
    ) || memo_length(
        control,
        MIN_SLOT + index,
        || bound_length(control, "min_size", index as usize),
        uses,
    )
}

/// `read` over the parsed size rule on `axis`, without cloning it.
pub(super) fn with_size<R>(
    control: &ResolvedControl,
    axis: Axis,
    read: impl FnOnce(Option<&Length>) -> R,
) -> R {
    memo_length(
        control,
        SIZE_SLOT + axis_index(axis) as u8,
        || Some(length(control, axis)),
        read,
    )
}

/// Whether the size on `axis` is `fill`.
pub(super) fn is_fill(control: &ResolvedControl, axis: Axis) -> bool {
    with_size(control, axis, |length| matches!(length, Some(Length::Fill)))
}

fn memo_length<R>(
    control: &ResolvedControl,
    slot: u8,
    read: impl FnOnce() -> Option<Length>,
    eval: impl FnOnce(Option<&Length>) -> R,
) -> R {
    let key = (std::ptr::from_ref(control).addr(), slot);
    measure::LENGTHS.with(|memo| {
        let mut memo = memo.borrow_mut();
        let length = memo.entry(key).or_insert_with(read);
        eval(length.as_ref())
    })
}

/// A control's size on `axis`. A missing or non-scalar element is `default`:
/// a label's text size, a ratio-scaled image's texture size, a stack panel's
/// summed children along its axis, else the parent's full extent.
fn length(control: &ResolvedControl, axis: Axis) -> Length {
    let explicit = control.properties.get("size").map(|size| match size {
        Value::Array(items) => items
            .get(axis_index(axis))
            .map_or(Length::Default, expr::length_from_value),
        _ => Length::Default,
    });
    match explicit {
        Some(Length::Default) | None if super::stack_axis(control) == Some(axis) => {
            expr::parse_length("100%c")
        }
        Some(length) => length,
        None => Length::Default,
    }
}

/// A bound rule on `index`, present only as an expression that builds a term: a
/// `%c` term builds one per child, so a childless control's `%c` builds none.
fn bound_length(control: &ResolvedControl, key: &str, index: usize) -> Option<Length> {
    let Value::Array(items) = control.properties.get(key)? else {
        return None;
    };
    let Length::Terms(terms) = expr::length_from_value(items.get(index)?) else {
        return None;
    };
    let sums_children = !control.children.is_empty() && !super::grid::has_template(control);
    let builds = |term: &expr::Term| {
        term.coeff != 0.0 && (term.unit != Unit::PercentChildren || sums_children)
    };
    terms.iter().any(builds).then_some(Length::Terms(terms))
}

/// The natural extent on `axis`: a label's text (wrapped at a known width), or a
/// ratio-scaled image's texture size, one default axis following the other.
fn natural(
    control: &ResolvedControl,
    axis: Axis,
    own: [Option<f64>; 2],
    width: Option<f64>,
    env: &LayoutEnv,
) -> Option<f64> {
    match control.control_type.as_deref() {
        Some("label") => label_extent(control, width, env).map(|extent| extent[axis_index(axis)]),
        Some("image") if scales_to_ratio(control) => {
            let [tw, th] = texture_size(control, env)?;
            let ratio = |numerator: f64, denominator: f64| {
                if denominator == 0.0 {
                    0.0
                } else {
                    numerator / denominator
                }
            };
            let other_default = with_size(control, other(axis), |length| {
                matches!(length, Some(Length::Default))
            });
            Some(match (axis, other_default) {
                (Axis::X, true) => tw,
                (Axis::Y, true) => th,
                (Axis::X, false) => ratio(own[1].unwrap_or(0.0), th) * tw,
                (Axis::Y, false) => ratio(own[0].unwrap_or(0.0), tw) * th,
            })
        }
        _ => None,
    }
}

fn scales_to_ratio(control: &ResolvedControl) -> bool {
    control.control_type.as_deref() == Some("image")
        && widgets::bound_bool(control, "default_size_scales_to_ratio") == Some(true)
}

fn texture_size(control: &ResolvedControl, env: &LayoutEnv) -> Option<[f64; 2]> {
    measure::natural(control, None, || {
        let path = control.properties.get("texture")?.as_str()?;
        env.textures.texture(path).map(|meta| meta.base_size)
    })
}

/// A label's text extent, wrapped at `width` when known, scaled by its font.
pub(super) fn label_extent(
    control: &ResolvedControl,
    width: Option<f64>,
    env: &LayoutEnv,
) -> Option<[f64; 2]> {
    if control.control_type.as_deref() != Some("label") {
        return None;
    }
    measure::natural(control, width, || {
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
    })
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

/// A label localizes its text unless `localize` is `false`.
pub(crate) fn localizes(control: &ResolvedControl) -> bool {
    control.properties.get("localize") != Some(&Value::Bool(false))
}

fn label_text(control: &ResolvedControl) -> String {
    match control.properties.get("text").and_then(Value::as_str) {
        // An unbound `#binding` has no literal extent.
        Some(text) if !text.starts_with('#') => text.to_owned(),
        _ => String::new(),
    }
}
