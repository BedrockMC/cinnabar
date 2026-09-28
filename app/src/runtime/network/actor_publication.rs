use bevy::{
    ecs::system::SystemParam,
    prelude::{Local, Projection, Res, ResMut, Resource, Time},
    time::Real,
};
use client_world::{LocalPlayerFeed, WorldStream};
use render::{
    ActorCullView, ActorMainWitness, ActorRenderFrame, ActorRenderScene, ActorRigFrameBuilder,
    ActorRigSubmission, HandItemAtlas, HandRigLight, HandRigScene,
    MAX_ACTOR_RENDER_DISTANCE_BLOCKS,
};

use super::{
    ActorFrameClock, ActorPresentationState, authoritative_local_actor_eye,
    dropped_items::DroppedItemPublisher, publish_local_actor_visibility,
};
use crate::{
    presentation::actors::{
        ActorRigPresentation, actor_rig_presentation, local_actor_presentation_for_visibility,
        local_diagnostic_presentation, rig_world_from_actor, select_actor_presentations_for_view,
        update_actor_rig_scene,
    },
    presentation::equipment::{
        EquipmentPresentation, EquipmentRuntime, FirstPersonArms, local_input, remote_input,
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

/// Rebuilds the scene's pack geometry and artwork for a new session, or restores the
/// startup artwork when a pack session ends.
fn apply_session_pack(
    scene: &mut ActorRenderScene,
    base: &render::ActorArtworkPages,
    pack: Option<&super::entity_pack::SessionEntityPack>,
    effective: &mut Option<render::ActorArtworkPages>,
) {
    if let Err(error) = scene.replace_pack_entities(pack.map(|pack| &*pack.assets)) {
        bevy::log::warn!(?error, "server pack entity geometry was not applied");
    }
    let pages = match pack {
        Some(pack) => base
            .clone()
            .with_pack_artwork(&pack.textures, &pack.bindings),
        None => base.clone(),
    };
    scene.configure_artwork(pages.clone());
    *effective = pack.map(|_| pages);
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
    /// The startup artwork plus the session's server-pack pages; `None` in a vanilla session.
    session_artwork: Local<'s, Option<render::ActorArtworkPages>>,
    hand_builder: ResMut<'w, HandRigBuilder>,
    hand_scene: ResMut<'w, HandRigScene>,
    hand_revision: Local<'s, u64>,
    local_skin: Res<'w, crate::player_skin::LocalPlayerSkin>,
    equipment: Option<ResMut<'w, EquipmentRuntime>>,
    ui: Option<Res<'w, crate::ui_runtime::UiRuntime>>,
    collisions: Option<Res<'w, crate::movement::PhysicsCollisionRegistries>>,
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
        mut session_artwork,
        mut hand_builder,
        mut hand_scene,
        mut hand_revision,
        local_skin,
        mut equipment,
        collisions,
        ui,
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
        let pack = session_id.and_then(|_| client_world.pack_entities.clone());
        if pack.is_some() || session_artwork.is_some() {
            apply_session_pack(&mut scene, &artwork, pack.as_deref(), &mut session_artwork);
        }
    }
    let artwork = session_artwork.as_ref().unwrap_or(&artwork);
    let step = actor_clock.advance(time.delta());
    let first_person = settings.perspective() == semantic_input::PerspectiveMode::FirstPerson;
    let mut local_feed =
        build_local_player_feed(&local_physics, view.rotation(), first_person, &local_skin);
    if let (Some(feed), Some(stream)) = (local_feed.as_mut(), client_world.stream.as_ref()) {
        // The local player's held items are client-owned; the rig's item queries read them here.
        let input = local_input(stream, ui.as_deref(), stream.local_player_runtime_id());
        feed.main_hand = input.main.map(|item| item.identifier);
        feed.off_hand = input.off.map(|item| item.identifier);
    }
    if let Some(stream) = client_world.stream.as_mut() {
        if let Some(equipment) = equipment.as_deref() {
            stream.set_item_use_durations(equipment.item_use_durations());
        }
        // Feed the client-authored local pose before the tick advance and rig read so the
        // local body/hand are driven by the shared rig, not the static fallback.
        if let Some(feed) = &local_feed {
            stream.sync_local_player_pose(feed);
        }
        let (yaw, pitch, _) = view.rotation().to_euler(bevy::math::EulerRot::YXZ);
        stream.set_actor_camera_rotation([
            -pitch.to_degrees(),
            (180.0 - yaw.to_degrees()).rem_euclid(360.0),
        ]);
        if let Some(collisions) = collisions.as_deref() {
            let samples = sample_actor_fluids(stream, collisions);
            stream.set_actor_fluids(&samples);
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
    // First person draws the player's own rig near the camera: the visible arms with every other
    // bone hidden, and a drawable held item on the posed `rightItem` bone. Anything not covered
    // (an undrawable item) leaves the CPU viewmodel in charge.
    let hand_source: Option<HandSource> = if first_person {
        canonical_local.clone().and_then(|presentation| {
            let stream = client_world.stream.as_ref()?;
            let equipment = equipment.as_deref_mut()?;
            let input = local_input(stream, ui.as_deref(), local_runtime_id);
            let arms = FirstPersonArms::for_hands(
                input.main.as_ref().map(|item| item.identifier.as_ref()),
                input.off.as_ref().map(|item| item.identifier.as_ref()),
            );
            let body = equipment.mask_first_person(&presentation.submission, arms);
            let item = input.main.as_ref().and_then(|item| {
                let layer = equipment.first_person_item(&presentation.submission, item)?;
                let page = usize::from(layer.location.page()).checked_sub(1)?;
                let page = artwork.pages().get(page)?;
                let (width, height) = page.dimensions();
                let atlas = HandItemAtlas {
                    width,
                    height,
                    layers: page.layers(),
                    rgba8: page.shared_pixels(),
                };
                Some((layer, atlas))
            });
            (body.is_some() || item.is_some()).then_some(HandSource {
                presentation,
                body,
                item,
            })
        })
    } else {
        None
    };
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
    // The visibility override rebuilds the local transform, so re-apply the death tip-over.
    let local_death = client_world
        .stream
        .as_ref()
        .and_then(|stream| stream.actor(local_runtime_id))
        .and_then(|actor| actor.status.death_progress(step.partial_tick));
    let local = local.map(|mut local| {
        local.submission.world_from_actor = crate::presentation::actors::death_tilted(
            local.submission.world_from_actor,
            local_death,
        );
        local
    });
    let camera_position = cull_view
        .as_ref()
        .map(|view| view.camera_position.to_array());
    let mut batch = select_actor_presentations_for_view(
        local_runtime_id,
        local_visible,
        local,
        remotes,
        cull_view,
    );
    let selected_count = batch.submissions.len();
    if let (Some(equipment), Some(stream)) =
        (equipment.as_deref_mut(), client_world.stream.as_ref())
    {
        // Equipment rides each selected body's pose, so culled bodies never build layers.
        let bodies = batch.submissions.clone();
        for body in &bodies {
            let runtime_id = body.input.identity.runtime_id;
            let input = if runtime_id == local_runtime_id {
                local_input(stream, ui.as_deref(), runtime_id)
            } else {
                remote_input(stream, runtime_id)
            };
            for layer in equipment.layers_for(body, &input) {
                batch
                    .artwork
                    .insert(layer.submission.input.identity, layer.location);
                batch.submissions.push(layer.submission);
            }
        }
    }
    // Layers were built above from the visible body, so hiding the body keeps armor and held items.
    if let Some(stream) = client_world.stream.as_ref() {
        for submission in &mut batch.submissions {
            let identity = submission.input.identity;
            if identity.layer == render::ACTOR_LAYER_BODY
                && stream
                    .actor(identity.runtime_id)
                    .is_some_and(|actor| actor.is_invisible())
            {
                submission.route = render::ActorRigRoute::NoDraw;
            }
        }
    }
    if let Some(equipment) = equipment.as_deref_mut() {
        for geometry in equipment.take_pending_geometries() {
            // A rejected mesh only leaves that item undrawn.
            let _ = hand_builder.0.insert_geometry(geometry.clone());
            let _ = scene.insert_geometry(geometry);
        }
    }
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
    dropped_items.publish(
        client_world.stream.as_ref(),
        camera_position,
        step.partial_tick,
    );
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

/// Samples the water and lava at each actor's body; an unreadable block reads as dry.
fn sample_actor_fluids(
    stream: &WorldStream,
    collisions: &crate::movement::PhysicsCollisionRegistries,
) -> Vec<(u64, bool, bool)> {
    use sim::{BlockPhysicsFlags, CollisionWorld};
    let world = sim::PaletteWorld::new(
        stream.collision_store(),
        collisions.registry(stream.network_id_mode()),
        stream.current_dimension(),
    );
    stream
        .actor_fluid_sample_points()
        .into_iter()
        .filter(|(_, position)| position.iter().all(|axis| axis.is_finite()))
        .map(|(runtime_id, position)| {
            // A hair above the feet, so a fish resting on the bed still samples its own water.
            let y = position[1] + 0.1;
            let block = [
                position[0].floor() as i32,
                y.floor() as i32,
                position[2].floor() as i32,
            ];
            let (mut water, mut lava) = (false, false);
            if let Ok(sample) = world.block_physics(block) {
                for layer in sample.layers.iter() {
                    let submerged = f64::from(y) < f64::from(block[1]) + layer.fluid_height_blocks;
                    water |= submerged && layer.flags.contains(BlockPhysicsFlags::WATER);
                    lava |= submerged && layer.flags.contains(BlockPhysicsFlags::LAVA);
                }
            }
            (runtime_id, water, lava)
        })
        .collect()
}

/// Builds and publishes the local player's first-person rig for the near-camera pass, or clears
/// it when not in first person, when the look FOV is unavailable, or when no skin resolved.
fn publish_hand_rig(
    builder: &mut ActorRigFrameBuilder,
    scene: &mut HandRigScene,
    revision: &mut u64,
    source: Option<HandSource>,
    fov_radians: Option<f32>,
    light: HandRigLight,
    partial_tick: f32,
) {
    let (Some(source), Some(fov)) = (source, fov_radians) else {
        scene.clear();
        return;
    };
    let Some(skin) = source.presentation.skin_rgba8.clone() else {
        scene.clear();
        return;
    };
    let placement = hand_camera_from_rig(source.presentation.model_scale);
    let mut submissions = Vec::new();
    if let Some(mut body) = source.body {
        body.world_from_actor = placement;
        // The hand skin is a single-layer array; the third-person layer index does not apply.
        body.texture_layer = 0;
        submissions.push(body);
    }
    let mut atlas = None;
    if let Some((layer, item_atlas)) = source.item {
        let mut item = layer.submission;
        item.world_from_actor = placement;
        item.texture_layer = layer.location.layer() | HAND_ITEM_LAYER_FLAG;
        submissions.push(item);
        atlas = Some(item_atlas);
    }
    let frame = builder.build(partial_tick, None, submissions);
    *revision = revision.wrapping_add(1).max(1);
    if scene.publish(frame, skin, light, fov, *revision) {
        scene.set_item_atlas(atlas);
    }
}

/// What the first-person pass draws: arm-masked body pose and/or a held item with its atlas page.
struct HandSource {
    presentation: ActorRigPresentation,
    body: Option<ActorRigSubmission>,
    item: Option<(EquipmentPresentation, HandItemAtlas)>,
}

/// Marks an instance's texture layer as an item-atlas layer for the first-person shader.
const HAND_ITEM_LAYER_FLAG: u32 = 0x8000_0000;

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
    let (sneaking, sprinting) = physics.latest_sneak_sprint().unwrap_or_default();
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
        main_hand: None,
        off_hand: None,
        teleported: false,
        first_person,
        sneaking,
        sprinting,
    })
}
