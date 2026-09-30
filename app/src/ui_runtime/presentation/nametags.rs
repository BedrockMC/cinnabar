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
/// The tag hangs this far above the top of the actor's bounding box, in blocks.
const HEAD_CLEARANCE: f32 = 0.7;
/// Bounding-box heights used when the actor publishes none: standing and sneaking players.
const DEFAULT_HEIGHT: f32 = 1.8;
const SNEAKING_HEIGHT: f32 = 1.5;
/// Entity metadata key of the bounding-box height.
const METADATA_HEIGHT: u32 = 54;
/// World size of one font design pixel (1.6 / 60 blocks); the tag is a fixed-size billboard.
const BLOCKS_PER_FONT_PIXEL: f32 = 1.6 / 60.0;
/// Font atlas texels per font design pixel.
const TEXELS_PER_FONT_PIXEL: f32 = 2.0;
const SCALE_STEPS_PER_UNIT: f32 = 16.0;
/// Plate height in font pixels per text line (`-1..9` around the 8 px glyph line).
const PLATE_LINE_PX: f32 = 10.0;
/// Plate colour is black at alpha 0.25.
const BOX_ALPHA: u8 = 64;
const SNEAK_TEXT_ALPHA: u8 = 128;
const ACTOR_FLAG_SNEAKING: u32 = 1;
const ACTOR_FLAG_INVISIBLE: u32 = 5;
const ACTOR_FLAG_SHOW_NAME: u32 = 14;
const ACTOR_FLAG_ALWAYS_SHOW_NAME: u32 = 15;
/// Entity metadata key forcing the name tag visible regardless of distance-to-crosshair rules.
const METADATA_ALWAYS_SHOW_NAMETAG: u32 = 81;
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

/// Feet-to-tag height: the published box height (already scaled by the server), else the default
/// player box times the metadata scale, plus the head clearance.
fn tag_height(actor: &ActorSnapshot) -> f32 {
    let box_height = match actor.metadata.get(&METADATA_HEIGHT) {
        Some(ActorMetadataValue::Float(height)) if height.is_finite() && *height > 0.0 => *height,
        _ if actor_flag(actor, ACTOR_FLAG_SNEAKING) => SNEAKING_HEIGHT * actor.render_scale(),
        _ => DEFAULT_HEIGHT * actor.render_scale(),
    };
    box_height + HEAD_CLEARANCE
}

/// Where the tag hangs: the actor's interpolated render position raised by [`tag_height`], so it
/// moves exactly as the rig does at this frame's `partial_tick`.
fn tag_world_position(actor: &ActorSnapshot, partial_tick: f32) -> Option<Vec3> {
    Some(
        Vec3::from_array(actor.interpolated_position(partial_tick.clamp(0.0, 1.0))?)
            + Vec3::Y * tag_height(actor),
    )
}

/// Projects `actor`'s tag, or `None` when it is out of range, invisible, hidden by wall
/// occlusion while sneaking, behind the camera or off the content rect. `is_occluded` takes the
/// tag's world position and reports whether a wall blocks the line from the camera.
#[allow(clippy::too_many_arguments)]
pub(super) fn project_nametag(
    actor: &ActorSnapshot,
    name: Arc<str>,
    camera: &Camera,
    camera_transform: &GlobalTransform,
    content_size: [f32; 2],
    safe_area: SafeArea,
    partial_tick: f32,
    is_occluded: impl FnOnce(Vec3) -> bool,
) -> Option<NametagAnchor> {
    if actor_flag(actor, ACTOR_FLAG_INVISIBLE) {
        return None;
    }
    let is_player = matches!(actor.kind, ActorKind::Player { .. });
    let always_key = matches!(
        actor.metadata.get(&METADATA_ALWAYS_SHOW_NAMETAG),
        Some(ActorMetadataValue::Byte(value)) if *value != 0
    );
    let always = is_player || always_key || actor_flag(actor, ACTOR_FLAG_ALWAYS_SHOW_NAME);
    if !always && !actor_flag(actor, ACTOR_FLAG_SHOW_NAME) {
        return None;
    }
    let sneaking = actor_flag(actor, ACTOR_FLAG_SNEAKING);
    let position = tag_world_position(actor, partial_tick)?;
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
    let occluded = is_occluded(position);
    if sneaking && occluded {
        return None;
    }
    let pixels_per_block = (point.y - above.y).abs();
    (x.is_finite() && y.is_finite() && pixels_per_block.is_finite() && pixels_per_block > 0.0)
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

/// Text scale that gives a font pixel its fixed world size, quantized so the layout cache sees a
/// bounded set of sizes.
fn text_scale(pixels_per_block: f32) -> UiScale {
    let ratio = BLOCKS_PER_FONT_PIXEL * pixels_per_block / TEXELS_PER_FONT_PIXEL;
    let stepped = (ratio * SCALE_STEPS_PER_UNIT).round() / SCALE_STEPS_PER_UNIT;
    UiScale::new(stepped.clamp(UiScale::MIN, UiScale::MAX)).unwrap_or_default()
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
    content_size: [f32; 2],
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
                scale: text_scale(anchor.pixels_per_block),
                font,
            })
            .map_err(UiPresentationError::Text)?;
        let [width, height] = layout.size_64().map(|value| value as f32 / 64.0);
        // The text hangs below the anchor; the plate is one font pixel wider on each side and
        // spans one pixel above to one below the 8 px glyph line.
        let font_px = TEXELS_PER_FONT_PIXEL * text_scale(anchor.pixels_per_block).get();
        let left = anchor.x - width / 2.0;
        let top = anchor.y;
        let plate = [
            left - font_px,
            top - font_px,
            left + width + font_px,
            top + PLATE_LINE_PX * font_px - font_px,
        ];
        if plate[2] < 0.0
            || plate[0] > content_size[0]
            || plate[3] < 0.0
            || plate[1] > content_size[1]
        {
            continue;
        }
        let faint = anchor.sneaking || anchor.occluded;
        let box_alpha = if faint { BOX_ALPHA / 2 } else { BOX_ALPHA };
        let id = UiNodeId::new(*next_id);
        *next_id = next_id.saturating_add(1);
        nodes.push(
            UiNode::new(id, None, rect(plate[0], plate[1], plate[2], plate[3])?).with_visual(
                UiVisual::Solid {
                    texture_page: solid_texture_page,
                    color: [0, 0, 0, box_alpha],
                },
            ),
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
                    shadow: TextShadow::None,
                },
            ),
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    // A doubled metadata scale doubles the default box the tag sits on.
    #[test]
    fn metadata_scale_raises_the_tag_without_a_published_box() {
        let pose = client_world::ActorPose {
            position: [0.0; 3],
            pitch: 0.0,
            yaw: 0.0,
            head_yaw: 0.0,
        };
        let mut actor = ActorSnapshot {
            unique_id: 1,
            runtime_id: 1,
            spawn_revision: 1,
            movement_revision: 1,
            kind: ActorKind::Entity {
                identifier: "test:npc".into(),
            },
            position: [0.0; 3],
            velocity: [0.0; 3],
            pitch: 0.0,
            yaw: 0.0,
            head_yaw: 0.0,
            previous_pose: pose,
            received_pose: pose,
            interpolation_ticks_remaining: 0,
            body_yaw: 0.0,
            on_ground: None,
            teleported: false,
            player_mode: None,
            source_tick: None,
            metadata: Default::default(),
            attributes: Default::default(),
            int_properties: Default::default(),
            float_properties: Default::default(),
            status: Default::default(),
        };
        let unscaled = tag_height(&actor);
        actor.metadata.insert(38, ActorMetadataValue::Float(2.0));
        assert_eq!(tag_height(&actor), 2.0 * DEFAULT_HEIGHT + HEAD_CLEARANCE);
        assert!(tag_height(&actor) > unscaled);
        // A server-published box already carries the scale.
        actor
            .metadata
            .insert(METADATA_HEIGHT, ActorMetadataValue::Float(3.6));
        assert_eq!(tag_height(&actor), 3.6 + HEAD_CLEARANCE);
    }

    #[test]
    fn text_scale_grows_with_proximity_and_stays_bounded() {
        let near = text_scale(400.0).get();
        let far = text_scale(20.0).get();
        assert!(near > far);
        assert!((UiScale::MIN..=UiScale::MAX).contains(&near));
        assert_eq!(text_scale(1.0).get(), UiScale::MIN);
        assert_eq!(text_scale(10_000.0).get(), UiScale::MAX);
    }

    // Plain text on a 0.25-alpha black plate that is one font pixel wider per side and
    // spans one pixel above to one below the 8 px line, hanging below the anchor.
    #[test]
    fn tag_is_unshadowed_text_on_a_padded_plate_below_the_anchor() {
        let font = super::super::tests::fixture_font();
        let metrics = TextMetrics::for_viewport([800, 600], ui::DpiScale::new(1.0).unwrap(), None);
        let anchor = NametagAnchor {
            x: 400.0,
            y: 300.0,
            name: "AB".into(),
            sneaking: false,
            occluded: false,
            pixels_per_block: 150.0,
            distance: 5.0,
        };
        let mut nodes = Vec::new();
        let mut next = 1;
        let mut layouts = TextLayoutCache::new(8, 1 << 20);
        append_nametag_nodes(
            &mut nodes,
            &mut next,
            &mut layouts,
            &font,
            metrics,
            0,
            &[anchor],
            [800.0, 600.0],
        )
        .unwrap();
        assert_eq!(nodes.len(), 2);
        let font_px = TEXELS_PER_FONT_PIXEL * text_scale(150.0).get();
        let (plate, text) = (nodes[0].bounds(), nodes[1].bounds());
        assert!(
            matches!(nodes[0].visual(), UiVisual::Solid { color, .. } if *color == [0, 0, 0, 64])
        );
        assert!(matches!(
            nodes[1].visual(),
            UiVisual::Text {
                shadow: TextShadow::None,
                ..
            }
        ));
        let close = |a: f32, b: f32| (a - b).abs() < 0.01;
        assert!(close(text.min().y(), 300.0));
        assert!(close(plate.min().x(), text.min().x() - font_px));
        assert!(close(plate.max().x(), text.max().x() + font_px));
        assert!(close(plate.min().y(), 300.0 - font_px));
        assert!(close(plate.max().y(), 300.0 + 9.0 * font_px));
    }

    // The tag anchors to the same interpolated position the rig draws at, per partial tick.
    #[test]
    fn tag_anchor_follows_the_interpolated_actor_position() {
        let actor = client_world::ActorSnapshot {
            unique_id: 1,
            runtime_id: 1,
            spawn_revision: 1,
            movement_revision: 1,
            kind: ActorKind::Player {
                uuid: [1; 16],
                username: "p".into(),
            },
            position: [4.0, 64.0, -2.0],
            velocity: [0.0; 3],
            pitch: 0.0,
            yaw: 0.0,
            head_yaw: 0.0,
            previous_pose: client_world::ActorPose {
                position: [2.0, 62.0, -2.0],
                pitch: 0.0,
                yaw: 0.0,
                head_yaw: 0.0,
            },
            received_pose: client_world::ActorPose {
                position: [4.0, 64.0, -2.0],
                pitch: 0.0,
                yaw: 0.0,
                head_yaw: 0.0,
            },
            interpolation_ticks_remaining: 0,
            body_yaw: 0.0,
            on_ground: Some(true),
            teleported: false,
            player_mode: None,
            source_tick: None,
            metadata: Default::default(),
            attributes: Default::default(),
            int_properties: Default::default(),
            float_properties: Default::default(),
            status: Default::default(),
        };
        for (partial, expected) in [(0.25, [2.5, 62.5]), (0.5, [3.0, 63.0]), (0.75, [3.5, 63.5])] {
            let anchor = tag_world_position(&actor, partial).unwrap();
            let rig = Vec3::from_array(actor.interpolated_position(partial).unwrap());
            assert_eq!(anchor - Vec3::Y * (DEFAULT_HEIGHT + HEAD_CLEARANCE), rig);
            assert!((anchor.x - expected[0]).abs() < 1e-5);
            assert!((anchor.y - (expected[1] + DEFAULT_HEIGHT + HEAD_CLEARANCE)).abs() < 1e-5);
        }
    }
}
