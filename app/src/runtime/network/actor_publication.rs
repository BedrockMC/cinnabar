use bevy::{
    ecs::system::SystemParam,
    prelude::{Local, Projection, Res, ResMut, Resource, Time},
    time::Real,
};
use client_world::{LocalPlayerFeed, WorldStream};
use render::{
    ActorCullView, ActorMainWitness, ActorRenderFrame, ActorRenderScene, ActorRigFrameBuilder,
    HandRigLight, HandRigScene, MAX_ACTOR_RENDER_DISTANCE_BLOCKS,
};

use super::{
    ActorFrameClock, dropped_items::DroppedItemPublisher, ActorPresentationState, authoritative_local_actor_eye,
    publish_local_actor_visibility,
};
use crate::{
    presentation::actors::{
        ActorRigPresentation, actor_rig_presentation, local_actor_presentation_for_visibility,
        local_diagnostic_presentation, rig_world_from_actor, select_actor_presentations_for_view,
        update_actor_rig_scene,
    },
    runtime::world::ClientWorld,
};

/// The local player's own first-person rig, built as a single instance placed in camera space.
#[derive(Resource)]
pub(crate) struct HandRigBuilder(pub(crate) ActorRigFrameBuilder);

impl HandRigBuilder {
    pub(crate) fn from_runtime_assets(
        assets: &assets::RuntimeEntityAssets,
    ) -> anyhow::Result<Self> {
        ActorRigFrameBuilder::from_runtime_assets(assets)
            .map(Self)
            .map_err(|error| {
                anyhow::anyhow!("prepare validated first-person hand rig geometry: {error:?}")
            })
    }
}

// Camera-local placement of the first-person rig. The rig is authored feet-up; it is faced away
// from the camera (arms reach toward the near plane) and dropped so the eye lands near the camera
// origin. These are initial estimates: the exact camera-to-rig offset is native-tuning work
// against the 26.30 client (the vanilla transform composes the camera matrix with data-driven
// animations rather than a single constant), while scale 0.9375 and the arm rest pose come from
// the samples and are carried by the evaluated pose itself.
const HAND_RIG_CAMERA_YAW_DEGREES: f32 = 180.0;
const HAND_RIG_CAMERA_OFFSET: [f32; 3] = [0.0, -1.5, 0.0];

fn hand_camera_from_rig(scale: f32) -> [[f32; 4]; 3] {
    rig_world_from_actor(HAND_RIG_CAMERA_OFFSET, HAND_RIG_CAMERA_YAW_DEGREES, scale)
}

#[derive(SystemParam)]
pub(crate) struct ActorFramePublication<'w, 's> {
    client_world: ResMut<'w, ClientWorld>,
    time: Res<'w, Time<Real>>,
    scene: ResMut<'w, ActorRenderScene>,
    frame: ResMut<'w, ActorRenderFrame>,
    published_session: Local<'s, Option<u64>>,
    actor_clock: Local<'s, ActorFrameClock>,
    presentation: ActorPresentationState<'w, 's>,
    artwork: Res<'w, render::ActorArtworkPages>,
    hand_builder: ResMut<'w, HandRigBuilder>,
    hand_scene: ResMut<'w, HandRigScene>,
    hand_revision: Local<'s, u64>,
    local_skin: Res<'w, crate::player_skin::LocalPlayerSkin>,
    dropped_items: DroppedItemPublisher<'w, 's>,
}

pub(crate) fn publish_actor_render_frame(params: ActorFramePublication) {
    let ActorFramePublication {
        mut client_world,
        time,
        mut scene,
        mut frame,
        mut published_session,
        mut actor_clock,
        presentation,
        artwork,
        mut hand_builder,
        mut hand_scene,
        mut hand_revision,
        local_skin,
        mut dropped_items,
    } = params;
    let ActorPresentationState {
        avatar,
        mut local_visibility,
        settings,
        view,
        local_physics,
        witness,
        camera,
    } = presentation;
    let session_id = client_world
        .stream
        .as_ref()
        .map(WorldStream::actor_session_id);
    if *published_session != session_id {
        scene.reset();
        actor_clock.reset();
        *published_session = session_id;
    }
    let step = actor_clock.advance(time.delta());
    let first_person = settings.perspective() == semantic_input::PerspectiveMode::FirstPerson;
    let local_feed =
        build_local_player_feed(&local_physics, view.rotation(), first_person, &local_skin);
    if let Some(stream) = client_world.stream.as_mut() {
        // Feed the client-authored local pose before the tick advance and rig read so the
        // local body/hand are driven by the shared rig, not the static fallback.
        if let Some(feed) = &local_feed {
            stream.sync_local_player_pose(feed);
        }
        stream.advance_actor_interpolation_ticks(step.ticks);
    }
    let authoritative_subject_eye = authoritative_local_actor_eye(
        local_physics.render_eye_position(),
        client_world
            .stream
            .as_ref()
            .map(|stream| stream.resolved_server_position().position),
    );
    publish_local_actor_visibility(
        &avatar,
        settings.perspective(),
        authoritative_subject_eye,
        view.rotation(),
        &mut local_visibility,
    );
    let cull_view = camera
        .single()
        .ok()
        .map(|(transform, projection)| ActorCullView {
            clip_from_world: projection.get_clip_from_view() * transform.to_matrix().inverse(),
            camera_position: transform.translation,
            max_distance: MAX_ACTOR_RENDER_DISTANCE_BLOCKS,
        });
    // The hand rig shares the main camera's vertical FOV; with no dynamic FOV modifiers yet this
    // is the base field of view before gameplay modifiers. Wire it to the base setting once modifiers land.
    let hand_camera_fov = camera
        .single()
        .ok()
        .and_then(|(_, projection)| match projection {
            Projection::Perspective(perspective) => Some(perspective.fov),
            _ => None,
        });
    let (local_runtime_id, actor_session_id, dimension, remotes, canonical_local, unrigged_actors) =
        client_world
            .stream
            .as_ref()
            .map(|stream| {
                let local_runtime_id = stream.local_player_runtime_id();
                let mut remotes = Vec::new();
                let mut canonical_local = None;
                let rigs = stream.actor_rigs();
                let unrigged_actors = stream.actor_count().saturating_sub(rigs.len());
                for rig in rigs {
                    let Some(actor) = stream.actor(rig.actor.runtime_id) else {
                        continue;
                    };
                    let profile = stream.actor_player_profile(rig.actor.runtime_id);
                    let presentation = if matches!(actor.kind, protocol::ActorKind::Player { .. }) {
                        actor_rig_presentation(&rig, actor, profile, step.partial_tick)
                    } else {
                        crate::presentation::actors::entity_rig_presentation(
                            &rig,
                            actor,
                            &artwork,
                            step.partial_tick,
                        )
                    };
                    let Some(presentation) = presentation else {
                        continue;
                    };
                    if rig.actor.runtime_id == local_runtime_id {
                        canonical_local = Some(presentation);
                    } else {
                        remotes.push(presentation);
                    }
                }
                (
                    local_runtime_id,
                    stream.actor_session_id(),
                    stream.current_dimension(),
                    remotes,
                    canonical_local,
                    unrigged_actors,
                )
            })
            .unwrap_or((0, 0, 0, Vec::new(), None, 0));
    // First person draws no near-camera rig: the earlier pass reused the full third-person body
    // rig shoved toward the camera, which occludes the view with the head/torso instead of an
    // arm. Vanilla first person draws an arm-only model; until that geometry exists the pass
    // stays dark so the world is visible.
    let hand_source: Option<ActorRigPresentation> = None;
    let visibility_snapshot = local_visibility.snapshot().copied();
    let (local_visible, local) = visibility_snapshot.map_or((false, None), |visibility| {
        if visibility.runtime_id() != local_runtime_id {
            return (false, None);
        }
        // The driven rig already carries the motion model's body yaw and head-over-body split,
        // so it is placed by its own transform. The static diagnostic is only a pre-rig fallback.
        let local = canonical_local.or_else(|| {
            let (yaw, pitch, _) = visibility.rotation().to_euler(bevy::math::EulerRot::YXZ);
            let yaw_degrees = (180.0 - yaw.to_degrees()).rem_euclid(360.0);
            let pitch_degrees = -pitch.to_degrees();
            let mut position = visibility.eye();
            position.y -= crate::local_player::LOCAL_AVATAR_EYE_HEIGHT_BLOCKS;
            let diagnostic = local_diagnostic_presentation(
                actor_session_id,
                dimension,
                visibility.runtime_id(),
                visibility.pose_generation(),
                position.to_array(),
                yaw_degrees,
                pitch_degrees,
            );
            local_actor_presentation_for_visibility(
                local_runtime_id,
                visibility.runtime_id(),
                None,
                diagnostic,
                yaw_degrees,
            )
        });
        (visibility.visible(), local)
    });
    let batch = select_actor_presentations_for_view(
        local_runtime_id,
        local_visible,
        local,
        remotes,
        cull_view,
    );
    let selected_count = batch.submissions.len();
    *frame = update_actor_rig_scene(&mut scene, step.partial_tick, batch).clone();
    witness.observe_main(ActorMainWitness {
        local_snapshot: visibility_snapshot.is_some(),
        local_visible,
        expected_runtime_id: local_runtime_id,
        visibility_runtime_id: visibility_snapshot.map_or(0, |snapshot| snapshot.runtime_id()),
        selected_count,
        local_route: frame
            .rig
            .manifest
            .iter()
            .find(|entry| entry.identity.runtime_id == local_runtime_id)
            .map(|entry| entry.route),
        frame_instances: frame.rig.instances.len(),
        frame_manifest: frame.rig.manifest.len(),
        skin_bytes: frame.skins_rgba8.len(),
        rejects: frame.rig.rejects,
        unrigged_actors,
    });
    let hand_light = client_world.stream.as_ref().map_or(
        HandRigLight {
            block_level: 0,
            sky_level: 0,
            daylight: 1.0,
            pad: 0,
        },
        |stream| {
            let (block, sky) = authoritative_subject_eye
                .map_or((0, 0), |eye| stream.light_level_at(eye.to_array()));
            HandRigLight {
                block_level: u32::from(block),
                sky_level: u32::from(sky),
                // Daylight is full until the celestial curve feeds the first-person pass; sky
                // light is already sampled per-position above.
                daylight: 1.0,
                pad: 0,
            }
        },
    );
    dropped_items.publish(client_world.stream.as_ref(), step.partial_tick);
    publish_hand_rig(
        &mut hand_builder.0,
        &mut hand_scene,
        &mut hand_revision,
        hand_source,
        hand_camera_fov,
        hand_light,
        step.partial_tick,
    );
}

/// Builds and publishes the local player's first-person rig for the near-camera pass, or clears
/// it when not in first person, when the look FOV is unavailable, or when no skin resolved.
fn publish_hand_rig(
    builder: &mut ActorRigFrameBuilder,
    scene: &mut HandRigScene,
    revision: &mut u64,
    source: Option<ActorRigPresentation>,
    fov_radians: Option<f32>,
    light: HandRigLight,
    partial_tick: f32,
) {
    let (Some(source), Some(fov)) = (source, fov_radians) else {
        scene.clear();
        return;
    };
    let Some(skin) = source.skin_rgba8 else {
        scene.clear();
        return;
    };
    let scale = source.model_scale;
    let mut submission = source.submission;
    submission.world_from_actor = hand_camera_from_rig(scale);
    // The hand skin is a single-layer array; the third-person layer index does not apply.
    submission.texture_layer = 0;
    let frame = builder.build(partial_tick, None, [submission]);
    *revision = revision.wrapping_add(1).max(1);
    scene.publish(frame, skin, light, fov, *revision);
}

/// Builds this frame's client-authored local-player feed from the predicted physics state and
/// the look pose. The yaw/pitch come from the look input (`LocalViewPose`), never the boomed
/// third-person camera. Returns `None` before physics or on any non-finite value.
fn build_local_player_feed(
    physics: &crate::movement::LocalPhysicsController,
    look: bevy::math::Quat,
    first_person: bool,
    local_skin: &crate::player_skin::LocalPlayerSkin,
) -> Option<LocalPlayerFeed> {
    let state = physics.state()?;
    let (yaw, pitch, _) = look.to_euler(bevy::math::EulerRot::YXZ);
    let yaw_degrees = (180.0 - yaw.to_degrees()).rem_euclid(360.0);
    let pitch_degrees = -pitch.to_degrees();
    let position = [
        state.position.x as f32,
        state.position.y as f32,
        state.position.z as f32,
    ];
    let velocity = [
        state.velocity.x as f32,
        state.velocity.y as f32,
        state.velocity.z as f32,
    ];
    if !position
        .iter()
        .chain(&velocity)
        .chain(&[yaw_degrees, pitch_degrees])
        .all(|value| value.is_finite())
    {
        return None;
    }
    Some(LocalPlayerFeed {
        // A real player-list echo overrides this; without one, the stream backs the local body
        // with the client's own uploaded skin under this stable local uuid.
        uuid: local_skin.local_uuid,
        username: std::sync::Arc::from(""),
        skin: local_skin.player_skin(),
        position,
        velocity,
        on_ground: state.on_ground,
        yaw: yaw_degrees,
        head_yaw: yaw_degrees,
        pitch: pitch_degrees,
        teleported: false,
        first_person,
    })
}
