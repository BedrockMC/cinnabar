//! Actor name tags as the vanilla client draws them: camera-facing world-space billboards of
//! text over translucent plates, laid out in font pixels (see `render::NametagScene`).

use std::sync::Arc;

use bevy::{
    camera::Camera,
    math::{Vec2, Vec3},
    prelude::GlobalTransform,
};
use client_world::ActorSnapshot;
use protocol::{ActorKind, ActorMetadataValue};
use render::{MAX_NAMETAG_RECORDS, NAMETAG_ATLAS_SIDE, NametagRecord, NametagScene};
use ui::{FONT_DESIGN_PIXEL_TEXELS, SafeArea, TextLayoutCache};

use super::nametag_atlas::{GlyphPage, NametagAtlas};
use super::retained_hud::{self, BelowNameAnchor};
use assets::RuntimeFontCatalog;

/// Vanilla's default nameplate render distance, used until an actor streams its own.
const DEFAULT_NAMEPLATE_DISTANCE: f32 = 64.0;
/// Entity metadata key of an actor's nameplate render distance.
const METADATA_NAMEPLATE_DISTANCE: u32 = 143;
/// Tags drawn per frame, nearest first.
pub(super) const MAX_PRESENTED_NAMETAGS: usize = 128;
/// The tag hangs this far above the top of the actor's bounding box, in blocks.
const HEAD_CLEARANCE: f32 = 0.7;
/// Each line past the first raises the whole tag by this much, in blocks.
const EXTRA_LINE_RAISE: f32 = 0.125;
/// Bounding-box heights used when the actor publishes none: standing and sneaking players.
const DEFAULT_HEIGHT: f32 = 1.8;
const SNEAKING_HEIGHT: f32 = 1.5;
/// Entity metadata key of the bounding-box height.
const METADATA_HEIGHT: u32 = 54;
/// Entity metadata keys of the name tag and the score tag appended below it.
const METADATA_NAME: u32 = 4;
const METADATA_SCORE_TAG: u32 = 84;
/// The score tag joins the name only within this many blocks of the camera.
const SCORE_TAG_DISTANCE: f32 = 10.0;
/// Line pitch in font pixels; each line's plate spans one pixel above to one below its glyphs.
const LINE_PITCH_PX: f32 = 10.0;
/// `BaseActorRenderer::NAME_TAG_BACKGROUND_COLOR`.
const PLATE_COLOR: [f32; 4] = [0.0, 0.0, 0.0, 0.25];
/// A sneaking actor's tag is depth tested and its text drawn at this alpha.
const SNEAK_TEXT_ALPHA: f32 = 0.125;
const ACTOR_FLAG_SNEAKING: u32 = 1;
const ACTOR_FLAG_INVISIBLE: u32 = 5;
const ACTOR_FLAG_SHOW_NAME: u32 = 14;
const ACTOR_FLAG_ALWAYS_SHOW_NAME: u32 = 15;
/// Entity metadata key forcing the name tag visible regardless of distance-to-crosshair rules.
const METADATA_ALWAYS_SHOW_NAMETAG: u32 = 81;
/// A mob flagged show-name (not always-show) presents its tag only near the view center.
const CROSSHAIR_RADIUS: f32 = 48.0;

/// One actor's tag: where its first line hangs and what it draws.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct NametagAnchor {
    /// World point of the tag's font-pixel origin, already raised for extra lines.
    pub(super) position: Vec3,
    /// Non-empty lines, top first.
    pub(super) lines: Vec<Arc<str>>,
    /// Drawn behind walls only where visible, as for a sneaking actor.
    pub(super) depth_tested: bool,
    pub(super) text_alpha: f32,
    pub(super) distance: f32,
}

fn actor_flag(actor: &ActorSnapshot, bit: u32) -> bool {
    matches!(
        actor.metadata.get(&0),
        Some(ActorMetadataValue::Flags(flags) | ActorMetadataValue::FlagsExtended(flags))
            if flags & (1_u64 << bit) != 0
    )
}

fn metadata_text(actor: &ActorSnapshot, key: u32) -> Option<&Arc<str>> {
    match actor.metadata.get(&key) {
        Some(ActorMetadataValue::String(text)) if !text.is_empty() => Some(text),
        _ => None,
    }
}

/// The actor's name tag: its streamed name, else a player's username.
fn tag_text(actor: &ActorSnapshot) -> Option<Arc<str>> {
    metadata_text(actor, METADATA_NAME)
        .cloned()
        .or_else(|| match &actor.kind {
            ActorKind::Player { username, .. } if !username.is_empty() => {
                Some(Arc::clone(username))
            }
            _ => None,
        })
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

/// Vanilla splits the text on newlines and drops empty lines.
fn tag_lines(name: &str, score: Option<&str>) -> Vec<Arc<str>> {
    name.split('\n')
        .chain(score.into_iter().flat_map(|score| score.split('\n')))
        .filter(|line| !line.is_empty())
        .map(Arc::from)
        .collect()
}

fn nameplate_distance(actor: &ActorSnapshot) -> f32 {
    match actor.metadata.get(&METADATA_NAMEPLATE_DISTANCE) {
        Some(ActorMetadataValue::Float(distance)) if distance.is_finite() => *distance,
        _ => DEFAULT_NAMEPLATE_DISTANCE,
    }
}

/// `actor`'s tag, or `None` when it is invisible, unnamed, past its nameplate distance, or a
/// show-name mob away from the crosshair.
/// `to_viewport` projects a world point to window logical px.
pub(crate) fn project_nametag(
    actor: &ActorSnapshot,
    eye: Vec3,
    to_viewport: &impl Fn(Vec3) -> Option<Vec2>,
    content_size: [f32; 2],
    safe_area: SafeArea,
    partial_tick: f32,
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
    let name = tag_text(actor)?;
    let feet = Vec3::from_array(actor.interpolated_position(partial_tick.clamp(0.0, 1.0))?);
    let distance = eye.distance(feet);
    if !distance.is_finite() || distance > nameplate_distance(actor) {
        return None;
    }
    let score = metadata_text(actor, METADATA_SCORE_TAG)
        .filter(|_| distance < SCORE_TAG_DISTANCE)
        .map(AsRef::as_ref);
    let lines = tag_lines(&name, score);
    if lines.is_empty() {
        return None;
    }
    let position = tag_world_position(actor, partial_tick)?
        + Vec3::Y * EXTRA_LINE_RAISE * (lines.len() - 1) as f32;
    if !always {
        let point = to_viewport(position)?;
        let center = [content_size[0] / 2.0, content_size[1] / 2.0];
        let (x, y) = (point.x - safe_area.left(), point.y - safe_area.top());
        if (x - center[0]).hypot(y - center[1]) > CROSSHAIR_RADIUS {
            return None;
        }
    }
    let sneaking = actor_flag(actor, ACTOR_FLAG_SNEAKING);
    Some(NametagAnchor {
        position,
        lines,
        depth_tested: sneaking,
        text_alpha: if sneaking { SNEAK_TEXT_ALPHA } else { 1.0 },
        distance,
    })
}

/// The frame's tag quads: see-through tags first, then depth-tested ones, each farthest first,
/// every tag its plates then its text.
pub(crate) fn build_nametag_scene<'p>(
    anchors: &[NametagAnchor],
    font: &RuntimeFontCatalog,
    layouts: &mut TextLayoutCache,
    atlas: &mut NametagAtlas,
    pages: &impl Fn(usize) -> Option<GlyphPage<'p>>,
) -> NametagScene {
    let mut ordered: Vec<&NametagAnchor> = anchors.iter().collect();
    ordered.sort_by(|a, b| {
        a.depth_tested
            .cmp(&b.depth_tested)
            .then(b.distance.total_cmp(&a.distance))
    });
    let lines: usize = ordered.iter().map(|anchor| anchor.lines.len()).sum();
    if !atlas.has_room_for(lines) {
        atlas.reset();
    }
    let side = NAMETAG_ATLAS_SIDE as f32;
    let texels = FONT_DESIGN_PIXEL_TEXELS as f32;
    let mut records = Vec::new();
    let mut see_through = 0;
    for anchor in ordered {
        let placed: Vec<_> = anchor
            .lines
            .iter()
            .filter_map(|line| atlas.line(line, font, layouts, pages))
            .collect();
        if placed.is_empty() || records.len() + placed.len() * 2 > MAX_NAMETAG_RECORDS {
            continue;
        }
        // Vanilla measures whole font pixels and pads the widest half-width by one pixel.
        let half = placed
            .iter()
            .map(|line| line.width_px.round() as i32 / 2)
            .max()
            .unwrap_or(0) as f32;
        for index in 0..placed.len() {
            let top = LINE_PITCH_PX * index as f32;
            records.push(NametagRecord {
                anchor: anchor.position.to_array(),
                text: 0,
                rect: [
                    -(half + 1.0),
                    top - 1.0,
                    half + 1.0,
                    top + LINE_PITCH_PX - 1.0,
                ],
                uv: [0.0, 0.0, -1.0, -1.0],
                color: PLATE_COLOR,
            });
        }
        for (index, line) in placed.iter().enumerate() {
            let left = -((line.width_px.round() as i32 / 2) as f32);
            let top = LINE_PITCH_PX * index as f32;
            let [x, y, width, height] = line.cell.map(|value| value as f32);
            let top = top + line.top_px;
            records.push(NametagRecord {
                anchor: anchor.position.to_array(),
                text: 1,
                rect: [left, top, left + width / texels, top + height / texels],
                uv: [x / side, y / side, (x + width) / side, (y + height) / side],
                color: [1.0, 1.0, 1.0, anchor.text_alpha],
            });
        }
        if !anchor.depth_tested {
            see_through = records.len();
        }
    }
    let (atlas, atlas_revision) = atlas.publish();
    NametagScene {
        records,
        see_through,
        atlas,
        atlas_revision,
    }
}

/// Tags for remote players and flagged mobs, nearest first; players with a below-name score get
/// the combined plate instead.
#[allow(clippy::too_many_arguments)]
pub(super) fn project_nametags(
    scoreboards: &ui::ScoreboardStore,
    stream: &client_world::WorldStream,
    camera: &Camera,
    camera_transform: &GlobalTransform,
    logical_size: [f32; 2],
    safe_area: SafeArea,
    partial_tick: f32,
    show_players: bool,
) -> Vec<NametagAnchor> {
    let content_size = [
        (logical_size[0] - safe_area.left() - safe_area.right()).max(0.0),
        (logical_size[1] - safe_area.top() - safe_area.bottom()).max(0.0),
    ];
    let mut anchors: Vec<NametagAnchor> = stream
        .remote_actors()
        .filter(|actor| show_players || !matches!(actor.kind, ActorKind::Player { .. }))
        .filter(|actor| {
            scoreboards
                .below_name_for_owner(&ui::ScoreOwner::Player(actor.unique_id))
                .or_else(|| {
                    scoreboards.below_name_for_owner(&ui::ScoreOwner::Entity(actor.unique_id))
                })
                .is_none()
        })
        .filter_map(|actor| {
            project_nametag(
                actor,
                camera_transform.translation(),
                &|point| camera.world_to_viewport(camera_transform, point).ok(),
                content_size,
                safe_area,
                partial_tick,
            )
        })
        .collect();
    anchors.sort_by(|a, b| a.distance.total_cmp(&b.distance));
    anchors.truncate(MAX_PRESENTED_NAMETAGS);
    anchors
}

/// Projects player names and their below-name scoreboard rows into the HUD.
pub(super) fn project_below_name_anchors(
    scoreboards: &ui::ScoreboardStore,
    stream: &client_world::WorldStream,
    camera: &Camera,
    camera_transform: &GlobalTransform,
    logical_size: [f32; 2],
    safe_area: SafeArea,
    partial_tick: f32,
) -> Vec<BelowNameAnchor> {
    let content_width = (logical_size[0] - safe_area.left() - safe_area.right()).max(0.0);
    let content_height = (logical_size[1] - safe_area.top() - safe_area.bottom()).max(0.0);
    stream
        .render_players()
        .into_iter()
        .filter_map(|(actor, _profile)| {
            let below_name = scoreboards
                .below_name_for_owner(&ui::ScoreOwner::Player(actor.unique_id))
                .or_else(|| {
                    scoreboards.below_name_for_owner(&ui::ScoreOwner::Entity(actor.unique_id))
                })?;
            let name = stream.actor_display_name(actor.unique_id)?;
            let position =
                Vec3::from_array(actor.interpolated_position(partial_tick)?) + Vec3::Y * 2.35;
            let viewport = camera.world_to_viewport(camera_transform, position).ok()?;
            let x = viewport.x - safe_area.left();
            let y = viewport.y - safe_area.top();
            (x.is_finite()
                && y.is_finite()
                && x >= 0.0
                && x <= content_width
                && y >= 0.0
                && y <= content_height)
                .then_some(BelowNameAnchor {
                    x,
                    y,
                    name,
                    score: below_name.0,
                    objective: below_name.1,
                })
        })
        .take(retained_hud::MAX_PRESENTED_BELOW_NAME_ROWS)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::super::nametag_atlas::font_page;
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

    fn anchor(lines: &[&str], depth_tested: bool, distance: f32) -> NametagAnchor {
        NametagAnchor {
            position: Vec3::new(0.0, 66.0, 0.0),
            lines: lines.iter().map(|line| Arc::from(*line)).collect(),
            depth_tested,
            text_alpha: if depth_tested { SNEAK_TEXT_ALPHA } else { 1.0 },
            distance,
        }
    }

    // Vanilla's layout: every line gets a 0.25-alpha black plate spanning the widest half-width
    // plus one pixel, one pixel above to one below its 10 px row; text is centred per line.
    #[test]
    fn tags_lay_out_vanilla_plates_and_centred_lines() {
        let font = super::super::tests::fixture_font();
        let mut layouts = TextLayoutCache::new(8, 1 << 20);
        let mut atlas = NametagAtlas::default();
        let scene = build_nametag_scene(
            &[anchor(&["ABCD", "A"], false, 5.0)],
            &font,
            &mut layouts,
            &mut atlas,
            &|page| font_page(&font, page),
        );
        assert_eq!(scene.records.len(), 4);
        assert_eq!(scene.see_through, 4);
        let (plates, text) = scene.records.split_at(2);
        let wide = atlas
            .line(&Arc::from("ABCD"), &font, &mut layouts, &|page| {
                font_page(&font, page)
            })
            .unwrap();
        let half = (wide.width_px.round() as i32 / 2) as f32;
        for (index, plate) in plates.iter().enumerate() {
            let top = LINE_PITCH_PX * index as f32;
            assert_eq!(
                plate.rect,
                [-(half + 1.0), top - 1.0, half + 1.0, top + 9.0]
            );
            assert_eq!(plate.color, PLATE_COLOR);
            assert_eq!(plate.text, 0);
        }
        assert_eq!(text[0].rect[0], -half);
        assert!(
            text[1].rect[0] > text[0].rect[0],
            "the short line is centred on its own"
        );
        assert_eq!(text[1].rect[1], LINE_PITCH_PX);
        assert!(
            text.iter()
                .all(|line| line.text == 1 && line.uv[2] > line.uv[0])
        );
        assert_eq!(scene.atlas.len(), 2);
        assert_eq!(scene.atlas[0].cell, wide.cell);
    }

    // See-through tags draw first and depth-tested (sneaking) ones after, each farthest first.
    #[test]
    fn see_through_tags_precede_depth_tested_ones_farthest_first() {
        let font = super::super::tests::fixture_font();
        let mut layouts = TextLayoutCache::new(8, 1 << 20);
        let mut atlas = NametagAtlas::default();
        let scene = build_nametag_scene(
            &[
                anchor(&["near"], false, 2.0),
                anchor(&["sneak"], true, 9.0),
                anchor(&["far"], false, 8.0),
            ],
            &font,
            &mut layouts,
            &mut atlas,
            &|page| font_page(&font, page),
        );
        assert_eq!(scene.see_through, 4);
        let far = atlas
            .line(&Arc::from("far"), &font, &mut layouts, &|page| {
                font_page(&font, page)
            })
            .unwrap();
        assert_eq!(
            scene.records[1].uv[0],
            far.cell[0] as f32 / NAMETAG_ATLAS_SIDE as f32
        );
        assert_eq!(scene.records[1].color[3], 1.0);
        assert_eq!(scene.records[5].color[3], SNEAK_TEXT_ALPHA);
    }

    #[test]
    fn lines_split_on_newlines_drop_empties_and_append_the_score() {
        let lines = tag_lines("\u{a7}eName\n\nTOUCH", Some("12 kills"));
        let lines: Vec<&str> = lines.iter().map(AsRef::as_ref).collect();
        assert_eq!(lines, ["\u{a7}eName", "TOUCH", "12 kills"]);
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
