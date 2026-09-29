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
    pub draw: Draw,
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

fn collect(
    node: &LaidOut,
    env: &LayoutEnv,
    out: &mut Vec<(i32, usize, DrawNode)>,
    order: &mut usize,
) {
    if !node.visible {
        return;
    }
    let visible_rect = node
        .clip_ratio
        .map(|ratio| clipped_rect(node.control, node.rect, ratio));
    for (dest, draw) in draws_for(node.control, node.rect, env) {
        let Some((dest, draw)) = crop(dest, draw, visible_rect) else {
            continue;
        };
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
                draw,
            },
        ));
        *order += 1;
    }
    for child in &node.children {
        collect(child, env, out, order);
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

fn text_draw(control: &ResolvedControl) -> Draw {
    let text = match control.properties.get("text").and_then(Value::as_str) {
        Some(text) => text.to_owned(),
        None => String::new(),
    };
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
    }
}

fn custom_draw(control: &ResolvedControl) -> Option<Draw> {
    let renderer = control.properties.get("renderer")?.as_str()?.to_owned();
    let data = control
        .properties
        .iter()
        .filter(|(key, _)| key.starts_with('#') || key.as_str() == "collection_index")
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
fn color_from_value(value: &Value, fallback: [u8; 4]) -> [u8; 4] {
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
            match (channel(0), channel(1), channel(2)) {
                (Some(r), Some(g), Some(b)) => [r, g, b, alpha],
                _ => fallback,
            }
        }
        Value::String(text) => named_color(text).unwrap_or(fallback),
        _ => fallback,
    }
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
            let (sx0, sx1) = (src_x[col], src_x[col + 1]);
            let (sy0, sy1) = (src_y[row], src_y[row + 1]);
            if dx1 - dx0 <= 0.0 || dy1 - dy0 <= 0.0 || sx1 - sx0 <= 0.0 || sy1 - sy0 <= 0.0 {
                continue;
            }
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

    #[test]
    fn color_floats_round_to_bytes() {
        let color = color_from_value(&serde_json::json!([0.3, 0.3, 0.3]), [1, 1, 1, 1]);
        assert_eq!(color, [77, 77, 77, 255]);
    }
}
