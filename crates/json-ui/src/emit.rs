//! Turn a laid-out tree into a flat, layer-ordered list of draw commands. Sprites
//! carry the texture path and a normalised source sub-rect; the atlas that maps a
//! path to a page and pixel UVs binds later. A nine-slice `image` emits up to nine
//! sprite quads (corners 1:1, edges stretched on one axis, centre on both); a zero
//! inset collapses that border so the neighbour stretches to the edge. A clipped
//! progress image (`clip_direction` + ratio) crops its quads and their UVs, and a
//! `custom` control emits an opaque [`Draw::Custom`] the caller renders itself.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::layout::{LaidOut, LayoutEnv, Rect};
use crate::sidecar::TextureMeta;
use crate::tree::ResolvedControl;

/// A normalised source sub-rect (0..1 of the texture) for a sprite quad.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct UvRect {
    pub u0: f32,
    pub v0: f32,
    pub u1: f32,
    pub v1: f32,
}

impl UvRect {
    pub fn full() -> Self {
        Self {
            u0: 0.0,
            v0: 0.0,
            u1: 1.0,
            v1: 1.0,
        }
    }
}

/// A rect derived by nine-slicing: where to draw and which source sub-rect to sample.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct SpriteQuad {
    pub dest: RectOut,
    pub uv: UvRect,
}

/// A serialisable rect; layout's [`Rect`] is copied into it for emit output.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct RectOut {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

impl From<Rect> for RectOut {
    fn from(rect: Rect) -> Self {
        Self {
            x: rect.x,
            y: rect.y,
            w: rect.w,
            h: rect.h,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum TextAlign {
    #[default]
    Left,
    Center,
    Right,
}

/// A single primitive: what the carrier binds to the atlas and font later.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Draw {
    Solid {
        color: [u8; 4],
    },
    Sprite {
        texture: String,
        uv: UvRect,
        color: [u8; 4],
    },
    Text {
        text: String,
        color: [u8; 4],
        shadow: bool,
        align: TextAlign,
        /// `font_scale_factor`: glyphs draw this many times their natural size.
        scale: f32,
        /// Whether the text is a language key or `%token` text (`localize`, default on).
        localize: bool,
    },
    /// A `custom` control (`renderer` names it, e.g. `inventory_item_renderer`)
    /// with its bound `#` values, drawn by the caller.
    Custom {
        renderer: String,
        data: BTreeMap<String, Value>,
    },
}

/// One positioned primitive, in final draw order.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DrawNode {
    /// The source control's instance name, so a bound node can be traced back.
    pub name: String,
    /// The source control's layout key (see [`crate::state`]).
    pub key: String,
    pub dest: RectOut,
    pub clip: RectOut,
    pub layer: i32,
    pub alpha: f32,
    /// Animations scaling `alpha`, evaluated by the caller at paint time.
    #[serde(default)]
    pub fades: Vec<crate::anim::Fade>,
    /// A sprite's `uv` flip-book, stepped by the caller at paint time.
    #[serde(default)]
    pub flip_book: Option<crate::anim::FlipBook>,
    /// Offset animations shifting `dest` and `clip`, evaluated at paint time.
    #[serde(default)]
    pub motions: crate::anim::Motions,
    pub draw: Draw,
    /// State children this node sits under, from [`emit_gated`]; see [`DrawNode::shown`].
    #[serde(default)]
    pub gates: Vec<StateGate>,
}

/// A state child (hover, pressed, …) of the control at `key`, shown under the
/// interaction states set in `mask`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StateGate {
    pub key: String,
    pub mask: u8,
}

impl DrawNode {
    /// Whether the node shows under `state`: every state child it sits under is
    /// the one its control shows.
    pub fn shown(&self, state: &crate::state::ViewState) -> bool {
        self.gates
            .iter()
            .all(|gate| gate.mask & (1 << crate::widgets::state_index(state, &gate.key)) != 0)
    }
}

impl DrawNode {
    /// `dest` and `clip` displaced by this node's offset animations at `now`.
    pub fn animated_rects(
        &self,
        now: f64,
        clocks: Option<&BTreeMap<String, f64>>,
    ) -> (RectOut, RectOut) {
        if self.motions.own.is_empty() && self.motions.clip.is_empty() {
            return (self.dest, self.clip);
        }
        let (own, clip) = self.motions.at(now, clocks);
        let shift = |rect: RectOut, by: [f64; 2]| RectOut {
            x: rect.x + by[0],
            y: rect.y + by[1],
            ..rect
        };
        (shift(self.dest, own), shift(self.clip, clip))
    }
}

/// Flatten a laid-out tree to draw commands, ordered by `layer` then document order.
/// Invisible controls and their descendants are dropped.
pub fn emit(root: &LaidOut, env: &LayoutEnv) -> Vec<DrawNode> {
    let mut nodes = Vec::new();
    let mut order = 0usize;
    collect(root, env, &mut nodes, &mut order);
    nodes.sort_by_key(|(layer, index, _)| (*layer, *index));
    nodes.into_iter().map(|(_, _, node)| node).collect()
}

/// [`emit`] of a tree laid out with no hover, press or focus, keeping every state
/// child's subtree gated by the states it shows under, so an interaction change
/// only filters nodes ([`DrawNode::shown`]) instead of laying out again.
pub fn emit_gated(root: &LaidOut, env: &LayoutEnv) -> Vec<DrawNode> {
    let mut nodes = Vec::new();
    let mut order = 0usize;
    let mut gates = Vec::new();
    collect_gated(
        root,
        env,
        &mut nodes,
        &mut order,
        &mut gates,
        &mut Vec::new(),
    );
    nodes.sort_by_key(|(layer, index, _)| (*layer, *index));
    nodes.into_iter().map(|(_, _, node)| node).collect()
}

fn collect_gated(
    node: &LaidOut,
    env: &LayoutEnv,
    out: &mut Vec<(i32, usize, DrawNode)>,
    order: &mut usize,
    gates: &mut Vec<StateGate>,
    pending: &mut Vec<(*const ResolvedControl, StateGate)>,
) {
    let gate = pending
        .iter()
        .rev()
        .find(|(target, _)| std::ptr::eq(*target, node.control))
        .map(|(_, gate)| gate.clone());
    // A state control the neutral state hides still emits, gated, if any state shows it.
    let shows_somewhere = gate.as_ref().is_some_and(|gate| gate.mask != 0);
    if !(node.visible || shows_somewhere) {
        return;
    }
    let gated = gate.is_some();
    gates.extend(gate);
    let first = out.len();
    emit_own(node, env, out, order);
    for (_, _, drawn) in &mut out[first..] {
        drawn.gates.clone_from(gates);
    }
    let before = pending.len();
    pending.extend(
        crate::widgets::state_targets(node.control, 0)
            .into_iter()
            .map(|target| {
                (
                    target.control as *const ResolvedControl,
                    StateGate {
                        key: node.key.clone(),
                        mask: target.mask,
                    },
                )
            }),
    );
    for child in &node.children {
        collect_gated(child, env, out, order, gates, pending);
    }
    pending.truncate(before);
    if gated {
        gates.pop();
    }
}

fn collect(
    node: &LaidOut,
    env: &LayoutEnv,
    out: &mut Vec<(i32, usize, DrawNode)>,
    order: &mut usize,
) {
    if !node.visible {
        return;
    }
    emit_own(node, env, out, order);
    for child in &node.children {
        collect(child, env, out, order);
    }
}

/// The node's own primitives, cropped to its progress clip.
fn emit_own(
    node: &LaidOut,
    env: &LayoutEnv,
    out: &mut Vec<(i32, usize, DrawNode)>,
    order: &mut usize,
) {
    let visible_rect = node
        .clip_ratio
        .map(|ratio| clipped_rect(node.control, node.rect, ratio));
    for (dest, draw) in draws_for(node.control, node.rect, env) {
        let Some((dest, draw)) = crop(dest, draw, visible_rect) else {
            continue;
        };
        if matches!(&draw, Draw::Text { text, .. } if text.is_empty()) {
            continue;
        }
        // A primitive wholly outside its clip (a scrolled-away cell) draws nothing;
        // a hover tooltip draws beside the pointer instead of in its rect.
        let clipped = dest.intersect(node.clip);
        let floats =
            matches!(&draw, Draw::Custom { renderer, .. } if renderer == "hover_text_renderer");
        if dest.w > 0.0 && dest.h > 0.0 && (clipped.w <= 0.0 || clipped.h <= 0.0) && !floats {
            continue;
        }
        out.push((
            node.layer,
            *order,
            DrawNode {
                name: node.control.name.clone(),
                key: node.key.clone(),
                dest: dest.into(),
                clip: node.clip.into(),
                layer: node.layer,
                alpha: node.alpha,
                fades: node.fades.clone(),
                motions: node.motions.clone(),
                flip_book: match &draw {
                    Draw::Sprite { texture, .. } => flip_book(node.control, texture, env),
                    _ => None,
                },
                draw,
                gates: Vec::new(),
            },
        ));
        *order += 1;
    }
}

/// The primitives a single control contributes at `rect`.
fn draws_for(control: &ResolvedControl, rect: Rect, env: &LayoutEnv) -> Vec<(Rect, Draw)> {
    // An empty texture (an unset binding) draws nothing, as in vanilla.
    if let Some(path) = control
        .properties
        .get("texture")
        .and_then(Value::as_str)
        .filter(|path| !path.is_empty())
    {
        return sprite_draws(control, rect, path, env);
    }
    match control.control_type.as_deref() {
        Some("label") => vec![(rect, text_draw(control))],
        Some("custom") => custom_draw(control)
            .map(|draw| vec![(rect, draw)])
            .unwrap_or_default(),
        Some("image") => solid_or_empty(control, rect),
        _ if is_fill(control) => solid_or_empty(control, rect),
        _ => Vec::new(),
    }
}

fn sprite_draws(
    control: &ResolvedControl,
    rect: Rect,
    path: &str,
    env: &LayoutEnv,
) -> Vec<(Rect, Draw)> {
    let color = color_of(control, [255, 255, 255, 255]);
    let meta = env.textures.texture(path);
    if let Some(source) = uv_rect(control, meta.as_ref()) {
        return vec![(
            rect,
            Draw::Sprite {
                texture: path.to_owned(),
                uv: source,
                color,
            },
        )];
    }
    if let (Some(axes), Some(meta)) = (tiled_axes(control), meta.as_ref()) {
        return tiles(rect, meta.base_size, axes)
            .into_iter()
            .map(|(dest, uv)| {
                (
                    dest,
                    Draw::Sprite {
                        texture: path.to_owned(),
                        uv,
                        color,
                    },
                )
            })
            .collect();
    }
    match meta {
        Some(meta) if meta.nineslice.is_some() => nine_slice(rect, &meta)
            .into_iter()
            .map(|quad| {
                (
                    Rect::new(quad.dest.x, quad.dest.y, quad.dest.w, quad.dest.h),
                    Draw::Sprite {
                        texture: path.to_owned(),
                        uv: quad.uv,
                        color,
                    },
                )
            })
            .collect(),
        _ => vec![(
            rect,
            Draw::Sprite {
                texture: path.to_owned(),
                uv: UvRect::full(),
                color,
            },
        )],
    }
}

/// The control's flip-book with its frame step normalised to `texture`'s width.
fn flip_book(
    control: &ResolvedControl,
    texture: &str,
    env: &LayoutEnv,
) -> Option<crate::anim::FlipBook> {
    let value = control.properties.get(crate::anim::FLIP_BOOK_KEY)?;
    let mut book: crate::anim::FlipBook = serde_json::from_value(value.clone()).ok()?;
    let width = env.textures.texture(texture)?.base_size[0];
    if book.frame_count <= 1 || width <= 0.0 {
        return None;
    }
    book.step_u = (book.frame_step / width) as f32;
    Some(book)
}

/// A literal `uv`/`uv_size` sub-rect of the texture, normalised.
fn uv_rect(control: &ResolvedControl, meta: Option<&TextureMeta>) -> Option<UvRect> {
    let pair = |key: &str| {
        let items = control.properties.get(key)?.as_array()?;
        Some([items.first()?.as_f64()?, items.get(1)?.as_f64()?])
    };
    let [u, v] = pair("uv")?;
    let [w, h] = pair("uv_size")?;
    let [bw, bh] = meta?.base_size;
    if bw <= 0.0 || bh <= 0.0 {
        return None;
    }
    Some(UvRect {
        u0: (u / bw) as f32,
        v0: (v / bh) as f32,
        u1: ((u + w) / bw) as f32,
        v1: ((v + h) / bh) as f32,
    })
}

/// `tiled`: `true` or `"xy"` repeats on both axes, `"x"`/`"y"` on one.
fn tiled_axes(control: &ResolvedControl) -> Option<[bool; 2]> {
    match control.properties.get("tiled")? {
        Value::Bool(true) => Some([true, true]),
        Value::String(axes) if axes == "x" => Some([true, false]),
        Value::String(axes) if axes == "y" => Some([false, true]),
        Value::String(axes) if axes == "xy" || axes == "true" => Some([true, true]),
        _ => None,
    }
}

/// Repeat a `base`-sized texture across `rect` on the tiled axes, cropping the
/// last tile; an untiled axis stretches.
fn tiles(rect: Rect, base: [f64; 2], axes: [bool; 2]) -> Vec<(Rect, UvRect)> {
    const MAX_TILES: usize = 4096;
    let spans = |start: f64, length: f64, tile: f64, tiled: bool| {
        if !tiled || tile <= 0.0 {
            return vec![(start, length, 1.0f32)];
        }
        let mut out = Vec::new();
        let mut at = 0.0;
        while at < length && out.len() < MAX_TILES {
            let span = tile.min(length - at);
            out.push((start + at, span, (span / tile) as f32));
            at += tile;
        }
        out
    };
    let mut quads = Vec::new();
    for (y, h, v) in spans(rect.y, rect.h, base[1], axes[1]) {
        for (x, w, u) in spans(rect.x, rect.w, base[0], axes[0]) {
            quads.push((
                Rect::new(x, y, w, h),
                UvRect {
                    u0: 0.0,
                    v0: 0.0,
                    u1: u,
                    v1: v,
                },
            ));
        }
    }
    quads
}

fn text_draw(control: &ResolvedControl) -> Draw {
    let mut text = match control.properties.get("text").and_then(Value::as_str) {
        Some(text) => text.to_owned(),
        None => String::new(),
    };
    // A selected edit box's text target draws its blinking caret after the text.
    if crate::widgets::bound_bool(control, crate::component::CARET_PROPERTY) == Some(true) {
        text.push('_');
    }
    let color = match control.properties.get("#color") {
        Some(value) => color_from_value(value, [255, 255, 255, 255]),
        None => color_of(control, [255, 255, 255, 255]),
    };
    Draw::Text {
        text,
        color,
        shadow: matches!(control.properties.get("shadow"), Some(Value::Bool(true))),
        align: alignment(control),
        scale: crate::layout::font_scale(control) as f32,
        localize: crate::layout::localizes(control),
    }
}

/// Plain properties a custom renderer reads besides its `#` bindings.
const CUSTOM_PROPERTIES: [&str; 4] = [
    "collection_index",
    "primary_color",
    "starting_rotation",
    "camera_tilt_degrees",
];

fn custom_draw(control: &ResolvedControl) -> Option<Draw> {
    let renderer = control.properties.get("renderer")?.as_str()?.to_owned();
    let data = control
        .properties
        .iter()
        .filter(|(key, _)| key.starts_with('#') || CUSTOM_PROPERTIES.contains(&key.as_str()))
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect();
    Some(Draw::Custom { renderer, data })
}

/// The part of `rect` a progress image keeps after clipping `ratio` of it away
/// toward `clip_direction` (the image stays pinned to the named side).
fn clipped_rect(control: &ResolvedControl, rect: Rect, ratio: f32) -> Rect {
    let keep = (1.0 - f64::from(ratio)).clamp(0.0, 1.0);
    match control
        .properties
        .get("clip_direction")
        .and_then(Value::as_str)
        .unwrap_or("left")
    {
        "right" => Rect::new(
            rect.x + rect.w * (1.0 - keep),
            rect.y,
            rect.w * keep,
            rect.h,
        ),
        "up" => Rect::new(rect.x, rect.y, rect.w, rect.h * keep),
        "down" => Rect::new(
            rect.x,
            rect.y + rect.h * (1.0 - keep),
            rect.w,
            rect.h * keep,
        ),
        "center" => {
            let (w, h) = (rect.w * keep, rect.h * keep);
            Rect::new(
                rect.x + (rect.w - w) * 0.5,
                rect.y + (rect.h - h) * 0.5,
                w,
                h,
            )
        }
        _ => Rect::new(rect.x, rect.y, rect.w * keep, rect.h),
    }
}

/// Crop a primitive to `visible`, scaling a sprite's UVs with its dest; a fully
/// clipped primitive yields `None`.
fn crop(dest: Rect, draw: Draw, visible: Option<Rect>) -> Option<(Rect, Draw)> {
    let Some(visible) = visible else {
        return Some((dest, draw));
    };
    let kept = dest.intersect(visible);
    if kept.w <= 0.0 || kept.h <= 0.0 {
        return None;
    }
    let draw = match draw {
        Draw::Sprite { texture, uv, color } if dest.w > 0.0 && dest.h > 0.0 => {
            let lerp_u = |x: f64| uv.u0 + (uv.u1 - uv.u0) * ((x - dest.x) / dest.w) as f32;
            let lerp_v = |y: f64| uv.v0 + (uv.v1 - uv.v0) * ((y - dest.y) / dest.h) as f32;
            Draw::Sprite {
                texture,
                uv: UvRect {
                    u0: lerp_u(kept.x),
                    v0: lerp_v(kept.y),
                    u1: lerp_u(kept.x + kept.w),
                    v1: lerp_v(kept.y + kept.h),
                },
                color,
            }
        }
        other => other,
    };
    Some((kept, draw))
}

fn solid_or_empty(control: &ResolvedControl, rect: Rect) -> Vec<(Rect, Draw)> {
    match control.properties.get("color") {
        Some(value) => vec![(
            rect,
            Draw::Solid {
                color: color_from_value(value, [255, 255, 255, 255]),
            },
        )],
        None => Vec::new(),
    }
}

fn is_fill(control: &ResolvedControl) -> bool {
    matches!(control.properties.get("fill"), Some(Value::Bool(true)))
}

fn alignment(control: &ResolvedControl) -> TextAlign {
    match control
        .properties
        .get("text_alignment")
        .and_then(Value::as_str)
    {
        Some("center") => TextAlign::Center,
        Some("right") => TextAlign::Right,
        _ => TextAlign::Left,
    }
}

fn color_of(control: &ResolvedControl, fallback: [u8; 4]) -> [u8; 4] {
    match control.properties.get("color") {
        Some(value) => color_from_value(value, fallback),
        None => fallback,
    }
}

/// JSON-UI colours are `[r, g, b]`/`[r, g, b, a]` floats in 0..1, a `#rrggbb` hex, or
/// one of a few names. Anything else falls back.
/// A JSON-UI colour value (`[r, g, b(, a)]` in 0..1, `#rrggbb`, or a name).
pub fn color_value(value: &Value) -> Option<[u8; 4]> {
    match value {
        Value::Array(items) if items.len() == 3 || items.len() == 4 => {
            let channel = |index: usize| {
                items
                    .get(index)
                    .and_then(Value::as_f64)
                    .map(|c| (c.clamp(0.0, 1.0) * 255.0).round() as u8)
            };
            let alpha = if items.len() == 4 {
                channel(3).unwrap_or(255)
            } else {
                255
            };
            Some([channel(0)?, channel(1)?, channel(2)?, alpha])
        }
        Value::String(text) => named_color(text),
        _ => None,
    }
}

fn color_from_value(value: &Value, fallback: [u8; 4]) -> [u8; 4] {
    color_value(value).unwrap_or(fallback)
}

fn named_color(text: &str) -> Option<[u8; 4]> {
    if let Some(hex) = text.strip_prefix('#')
        && hex.len() == 6
        && let Ok(rgb) = u32::from_str_radix(hex, 16)
    {
        return Some([(rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8, 255]);
    }
    match text {
        "white" => Some([255, 255, 255, 255]),
        "black" => Some([0, 0, 0, 255]),
        "gray" | "grey" => Some([128, 128, 128, 255]),
        "red" => Some([255, 0, 0, 255]),
        "green" => Some([0, 255, 0, 255]),
        "blue" => Some([0, 0, 255, 255]),
        _ => None,
    }
}

/// Split `dest` into up to nine sprite quads per `meta`'s nine-slice insets. Without
/// insets (or with a degenerate `base_size`) a single full-texture quad is returned.
pub fn nine_slice(dest: Rect, meta: &TextureMeta) -> Vec<SpriteQuad> {
    let [bw, bh] = meta.base_size;
    let Some(insets) = meta.nineslice else {
        return vec![full_quad(dest)];
    };
    if bw <= 0.0 || bh <= 0.0 {
        return vec![full_quad(dest)];
    }

    let (src_l, src_r) = fit(insets.left, insets.right, bw);
    let (src_t, src_b) = fit(insets.top, insets.bottom, bh);
    let (dst_l, dst_r) = fit(insets.left, insets.right, dest.w);
    let (dst_t, dst_b) = fit(insets.top, insets.bottom, dest.h);

    let src_x = [0.0, src_l, bw - src_r, bw];
    let src_y = [0.0, src_t, bh - src_b, bh];
    let dst_x = [
        dest.x,
        dest.x + dst_l,
        dest.x + dest.w - dst_r,
        dest.x + dest.w,
    ];
    let dst_y = [
        dest.y,
        dest.y + dst_t,
        dest.y + dest.h - dst_b,
        dest.y + dest.h,
    ];

    let mut quads = Vec::with_capacity(9);
    for row in 0..3 {
        for col in 0..3 {
            let (dx0, dx1) = (dst_x[col], dst_x[col + 1]);
            let (dy0, dy1) = (dst_y[row], dst_y[row + 1]);
            if dx1 - dx0 <= 0.0 || dy1 - dy0 <= 0.0 {
                continue;
            }
            // Insets meeting in the middle (a 2x2 texture sliced at 1) leave no
            // source span; the stretched region samples the texel line there.
            let (sx0, sx1) = texel_span(src_x[col], src_x[col + 1], bw);
            let (sy0, sy1) = texel_span(src_y[row], src_y[row + 1], bh);
            quads.push(SpriteQuad {
                dest: RectOut {
                    x: dx0,
                    y: dy0,
                    w: dx1 - dx0,
                    h: dy1 - dy0,
                },
                uv: UvRect {
                    u0: (sx0 / bw) as f32,
                    v0: (sy0 / bh) as f32,
                    u1: (sx1 / bw) as f32,
                    v1: (sy1 / bh) as f32,
                },
            });
        }
    }
    quads
}

/// A source span, widened to the one texel at its position when empty.
fn texel_span(start: f64, end: f64, size: f64) -> (f64, f64) {
    if end > start {
        return (start, end);
    }
    let low = (start - 0.5).clamp(0.0, (size - 1.0).max(0.0));
    (low, (low + 1.0).min(size))
}

fn full_quad(dest: Rect) -> SpriteQuad {
    SpriteQuad {
        dest: dest.into(),
        uv: UvRect::full(),
    }
}

/// Two opposing insets clamped to a total; when they overflow they shrink in
/// proportion so the centre never goes negative.
fn fit(a: f64, b: f64, total: f64) -> (f64, f64) {
    let sum = a + b;
    if sum <= total || sum <= 0.0 {
        (a, b)
    } else {
        (a * total / sum, b * total / sum)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sidecar::NineSlice;

    #[test]
    fn plain_texture_is_one_full_quad() {
        let meta = TextureMeta {
            base_size: [64.0, 64.0],
            nineslice: None,
        };
        let quads = nine_slice(Rect::new(0.0, 0.0, 100.0, 50.0), &meta);
        assert_eq!(quads.len(), 1);
        assert_eq!(quads[0].uv, UvRect::full());
    }

    #[test]
    fn zero_edge_collapses_its_row() {
        // top=0 removes the whole top row: 6 quads, not 9.
        let meta = TextureMeta {
            base_size: [3.0, 2.0],
            nineslice: Some(NineSlice {
                left: 1.0,
                top: 0.0,
                right: 1.0,
                bottom: 1.0,
            }),
        };
        let quads = nine_slice(Rect::new(0.0, 0.0, 30.0, 20.0), &meta);
        assert_eq!(quads.len(), 6);
        // The middle row starts at the top edge (y = 0) since the top inset is gone.
        assert!(quads.iter().all(|q| q.dest.y >= 0.0));
        assert_eq!(quads[0].dest.y, 0.0);
    }

    // A 2x2 texture sliced at 1 still fills its centre.
    #[test]
    fn meeting_insets_stretch_the_middle_texel() {
        let meta = TextureMeta {
            base_size: [2.0, 2.0],
            nineslice: Some(NineSlice {
                left: 1.0,
                top: 1.0,
                right: 1.0,
                bottom: 1.0,
            }),
        };
        let quads = nine_slice(Rect::new(0.0, 0.0, 30.0, 9.0), &meta);
        assert_eq!(quads.len(), 9);
        let centre = quads[4];
        assert_eq!([centre.dest.w, centre.dest.h], [28.0, 7.0]);
        assert_eq!([centre.uv.u0, centre.uv.u1], [0.25, 0.75]);
    }

    #[test]
    fn color_floats_round_to_bytes() {
        let color = color_from_value(&serde_json::json!([0.3, 0.3, 0.3]), [1, 1, 1, 1]);
        assert_eq!(color, [77, 77, 77, 255]);
    }
}
