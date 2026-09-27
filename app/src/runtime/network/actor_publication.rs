use bevy::{
    ecs::system::SystemParam,
    prelude::{Local, Res, ResMut, Time},
    time::Real,
};
use client_world::WorldStream;
use render::{
    ActorCullView, ActorMainWitness, ActorRenderFrame, ActorRenderScene,
    MAX_ACTOR_RENDER_DISTANCE_BLOCKS,
};

use super::{
    ActorFrameClock, ActorPresentationState, authoritative_local_actor_eye,
    publish_local_actor_visibility,
};
use crate::{
    presentation::actors::{
        actor_rig_presentation, local_actor_presentation_for_visibility,
        local_diagnostic_presentation, select_actor_presentations_for_view, update_actor_rig_scene,
    },
    runtime::world::ClientWorld,
};

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
    if let Some(stream) = client_world.stream.as_mut() {
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
    let visibility_snapshot = local_visibility.snapshot().copied();
    let (local_visible, local) = visibility_snapshot.map_or((false, None), |visibility| {
        if visibility.runtime_id() != local_runtime_id {
            return (false, None);
        }
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
        let local = local_actor_presentation_for_visibility(
            local_runtime_id,
            visibility.runtime_id(),
            canonical_local,
            diagnostic,
        );
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
}
