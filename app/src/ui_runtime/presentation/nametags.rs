//! World-projected player nametags: distance-scaled text over a translucent box.

use std::sync::Arc;

use bevy::{camera::Camera, math::Vec3, prelude::GlobalTransform};
use client_world::ActorSnapshot;
use protocol::{ActorKind, ActorMetadataValue};
use ui::{
    SafeArea, TextLayoutCache, TextLayoutRequest, TextShadow, TextStyle, UiNode, UiNodeId, UiScale,
    UiVisual,
};

use super::{TextMetrics, UiPresentationError, bounded_visible_text, rect};
use assets::RuntimeFontCatalog;

/// Nametags are dropped beyond this many blocks from the camera.
const MAX_NAMETAG_DISTANCE: f32 = 64.0;
/// Rows kept per frame so a crowded server cannot flood the retained tree.
pub(super) const MAX_PRESENTED_NAMETAGS: usize = 32;
/// Head-top offset the tag hangs above, in blocks.
const HEAD_OFFSET: f32 = 2.35;
/// World height of one 9 GUI px text line, in blocks. Needs independent measurement.
const LINE_WORLD_HEIGHT: f32 = 0.225;
/// Logical px of a text line at `UiScale` 1.0 (18 font texels).
const LINE_LOGICAL_AT_UNIT_SCALE: f32 = 18.0;
const SCALE_STEPS_PER_UNIT: f32 = 16.0;
/// A tag never grows past this multiple of the HUD text scale, however close the player is.
const MAX_SCALE_OVER_HUD: f32 = 2.0;
const BOX_PADDING: f32 = 2.0;
const BOX_ALPHA: u8 = 64;
const SNEAK_TEXT_ALPHA: u8 = 128;
const ACTOR_FLAG_SNEAKING: u32 = 1;
const ACTOR_FLAG_INVISIBLE: u32 = 5;
const ACTOR_FLAG_SHOW_NAME: u32 = 14;
const ACTOR_FLAG_ALWAYS_SHOW_NAME: u32 = 15;
/// A mob flagged show-name (not always-show) presents its tag only near the view center.
const CROSSHAIR_RADIUS: f32 = 48.0;
/// See-through tags behind walls are faint.
const OCCLUDED_TEXT_ALPHA: u8 = 48;

/// One player's tag position in safe-content logical px.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct NametagAnchor {
    pub(super) x: f32,
    pub(super) y: f32,
    pub(super) name: Arc<str>,
    pub(super) sneaking: bool,
    /// Line of sight to the actor is blocked; the tag shows only as a faint see-through.
    pub(super) occluded: bool,
    /// Logical px covered by one world block at the tag, which sets the text size.
    pub(super) pixels_per_block: f32,
    pub(super) distance: f32,
}

fn actor_flag(actor: &ActorSnapshot, bit: u32) -> bool {
    matches!(
        actor.metadata.get(&0),
        Some(ActorMetadataValue::Flags(flags) | ActorMetadataValue::FlagsExtended(flags))
            if flags & (1_u64 << bit) != 0
    )
}

/// Projects `actor`'s tag, or `None` when it is out of range, invisible, hidden by wall
/// occlusion while sneaking, behind the camera or off the content rect. `is_occluded` takes the
/// tag's world position and reports whether a wall blocks the line from the camera.
pub(super) fn project_nametag(
    actor: &ActorSnapshot,
    name: Arc<str>,
    camera: &Camera,
    camera_transform: &GlobalTransform,
    content_size: [f32; 2],
    safe_area: SafeArea,
    is_occluded: impl FnOnce(Vec3) -> bool,
) -> Option<NametagAnchor> {
    if actor_flag(actor, ACTOR_FLAG_INVISIBLE) {
        return None;
    }
    let is_player = matches!(actor.kind, ActorKind::Player { .. });
    let always = is_player || actor_flag(actor, ACTOR_FLAG_ALWAYS_SHOW_NAME);
    if !always && !actor_flag(actor, ACTOR_FLAG_SHOW_NAME) {
        return None;
    }
    let position = Vec3::from_array(actor.position) + Vec3::Y * HEAD_OFFSET;
    let distance = camera_transform.translation().distance(position);
    if !distance.is_finite() || distance > MAX_NAMETAG_DISTANCE {
        return None;
    }
    let point = camera.world_to_viewport(camera_transform, position).ok()?;
    let above = camera
        .world_to_viewport(camera_transform, position + Vec3::Y)
        .ok()?;
    let x = point.x - safe_area.left();
    let y = point.y - safe_area.top();
    if !always {
        let center = [content_size[0] / 2.0, content_size[1] / 2.0];
        if (x - center[0]).hypot(y - center[1]) > CROSSHAIR_RADIUS {
            return None;
        }
    }
    let sneaking = actor_flag(actor, ACTOR_FLAG_SNEAKING);
    let occluded = is_occluded(position);
    if sneaking && occluded {
        return None;
    }
    let pixels_per_block = (point.y - above.y).abs();
    (x.is_finite()
        && y.is_finite()
        && pixels_per_block.is_finite()
        && pixels_per_block > 0.0
        && (0.0..=content_size[0]).contains(&x)
        && (0.0..=content_size[1]).contains(&y))
    .then_some(NametagAnchor {
        x,
        y,
        name,
        sneaking,
        occluded,
        pixels_per_block,
        distance,
    })
}

/// Text scale that makes a line span `LINE_WORLD_HEIGHT` blocks, at most `cap`, quantized so the
/// layout cache sees a bounded set of sizes.
fn text_scale(pixels_per_block: f32, cap: f32) -> UiScale {
    let ratio = LINE_WORLD_HEIGHT * pixels_per_block / LINE_LOGICAL_AT_UNIT_SCALE;
    let stepped = (ratio * SCALE_STEPS_PER_UNIT).round() / SCALE_STEPS_PER_UNIT;
    UiScale::new(stepped.min(cap).clamp(UiScale::MIN, UiScale::MAX)).unwrap_or_default()
}

/// Appends nametags farthest first so nearer tags draw over farther ones.
#[allow(clippy::too_many_arguments)]
pub(super) fn append_nametag_nodes(
    nodes: &mut Vec<UiNode>,
    next_id: &mut u32,
    layouts: &mut TextLayoutCache,
    font: &RuntimeFontCatalog,
    metrics: TextMetrics,
    solid_texture_page: u16,
    anchors: &[NametagAnchor],
) -> Result<(), UiPresentationError> {
    let mut ordered: Vec<&NametagAnchor> = anchors.iter().take(MAX_PRESENTED_NAMETAGS).collect();
    ordered.sort_by(|a, b| b.distance.total_cmp(&a.distance));
    for anchor in ordered {
        let layout = layouts
            .layout(TextLayoutRequest {
                text: bounded_visible_text(&anchor.name),
                style: TextStyle::default(),
                width_64: 512 * 64,
                line_height_64: metrics.line_height_64,
                baseline_64: metrics.baseline_64,
                scale: text_scale(
                    anchor.pixels_per_block,
                    metrics.scale.get() * MAX_SCALE_OVER_HUD,
                ),
                font,
            })
            .map_err(UiPresentationError::Text)?;
        let [width, height] = layout.size_64().map(|value| value as f32 / 64.0);
        let left = anchor.x - width / 2.0;
        let top = anchor.y - height;
        let faint = anchor.sneaking || anchor.occluded;
        let box_alpha = if faint { BOX_ALPHA / 2 } else { BOX_ALPHA };
        let id = UiNodeId::new(*next_id);
        *next_id = next_id.saturating_add(1);
        nodes.push(
            UiNode::new(
                id,
                None,
                rect(
                    left - BOX_PADDING,
                    top - BOX_PADDING,
                    left + width + BOX_PADDING,
                    top + height + BOX_PADDING,
                )?,
            )
            .with_visual(UiVisual::Solid {
                texture_page: solid_texture_page,
                color: [0, 0, 0, box_alpha],
            }),
        );
        let id = UiNodeId::new(*next_id);
        *next_id = next_id.saturating_add(1);
        let alpha = if anchor.occluded {
            OCCLUDED_TEXT_ALPHA
        } else if anchor.sneaking {
            SNEAK_TEXT_ALPHA
        } else {
            255
        };
        nodes.push(
            UiNode::new(id, None, rect(left, top, left + width, top + height)?).with_visual(
                UiVisual::Text {
                    layout,
                    color: [255, 255, 255, alpha],
                    shadow: if faint {
                        TextShadow::None
                    } else {
                        metrics.shadow()
                    },
                },
            ),
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_scale_grows_with_proximity_and_stays_bounded() {
        let near = text_scale(400.0, 8.0).get();
        let far = text_scale(20.0, 8.0).get();
        assert!(near > far);
        assert!((UiScale::MIN..=UiScale::MAX).contains(&near));
        assert_eq!(text_scale(1.0, 8.0).get(), UiScale::MIN);
        assert_eq!(text_scale(10_000.0, 2.0).get(), 2.0);
    }
}
