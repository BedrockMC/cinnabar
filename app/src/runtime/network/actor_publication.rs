use std::sync::Arc;

use bevy::{
    ecs::system::SystemParam,
    math::Mat4,
    prelude::{Local, Projection, Res, ResMut, Resource, Time},
    time::Real,
};
use client_world::{LocalItemUse, LocalPlayerFeed, WorldStream};
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
    melee::SwingTracker,
    presentation::actors::{
        ActorRigPresentation, actor_rig_presentation, local_actor_presentation_for_visibility,
        local_diagnostic_presentation, rig_world_from_actor, select_actor_presentations_for_view,
        update_actor_rig_scene,
    },
    presentation::equipment::{
        EquipmentPresentation, EquipmentRuntime, FirstPersonArms, FirstPersonHand, FirstPersonItem,
        StagedSessionIcons, local_input, remote_input,
    },
    runtime::world::ClientWorld,
};

/// The frame fraction the actor rigs interpolate at, for overlays anchored to actors.
#[derive(Resource, Default)]
pub(crate) struct ActorFramePartialTick(pub(crate) f32);

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

/// Vertical FOV of the first-person pass; underwater and death-camera narrowing are not modelled.
pub(crate) const HAND_FOV_DEGREES: f32 = 70.0;

/// Vanilla draws the first-person rig in view space as a zero-yaw actor, feet one eye height
/// below the camera; the pack's first-person arm offsets are authored for that facing.
fn hand_camera_from_rig(scale: f32, motion: Mat4) -> [[f32; 4]; 3] {
    let rows = rig_world_from_actor(
        [
            0.0,
            -crate::local_player::LOCAL_AVATAR_EYE_HEIGHT_BLOCKS,
            0.0,
        ],
        0.0,
        scale,
    );
    let placement = Mat4::from_cols_array_2d(&[
        [rows[0][0], rows[1][0], rows[2][0], 0.0],
        [rows[0][1], rows[1][1], rows[2][1], 0.0],
        [rows[0][2], rows[1][2], rows[2][2], 0.0],
        [rows[0][3], rows[1][3], rows[2][3], 1.0],
    ]);
    let composed = (motion * placement).transpose().to_cols_array_2d();
    [composed[0], composed[1], composed[2]]
}

/// Rebuilds the scene's pack geometry and artwork for a new session, or restores the
/// startup artwork when a pack session ends.
fn apply_session_pack(
    scene: &mut ActorRenderScene,
    base: &render::ActorArtworkPages,
    pack: Option<&super::entity_pack::SessionEntityPack>,
    session_icons: Option<StagedSessionIcons>,
    effective: &mut Option<render::ActorArtworkPages>,
    equipment: Option<&mut EquipmentRuntime>,
) -> (
    Option<StagedSessionIcons>,
    Vec<Option<render::ActorArtworkLocation>>,
) {
    if let Err(error) = scene.replace_pack_entities(pack.map(|pack| &*pack.assets)) {
        bevy::log::warn!(?error, "server pack entity geometry was not applied");
    }
    let mut pages = match pack {
        Some(pack) => base
            .clone()
            .with_pack_artwork(&pack.textures, &pack.bindings),
        None => base.clone(),
    };
    let mut layer = None;
    let mut geometries = Vec::new();
    if let Some(pack) = pack
        && let Some(catalog) = &pack.equipment
    {
        let (extended, locations) =
            pages.with_equipment_rasters(&EquipmentRuntime::pack_rasters(catalog));
        pages = extended;
        geometries = EquipmentRuntime::pack_geometries(&pack.assets, catalog);
        layer = Some((Arc::clone(&pack.assets), Arc::clone(catalog), locations));
    }
    if let Err(error) = scene.replace_pack_equipment(geometries) {
        bevy::log::warn!(?error, "server pack equipment geometry was not applied");
    }
    if let Some(equipment) = equipment {
        equipment.set_pack_layer(layer);
    }
    let mut icon_locations = Vec::new();
    if let Some(icons) = &session_icons {
        let (extended, locations) = pages.with_equipment_rasters(icons.rasters());
        pages = extended;
        icon_locations = locations;
    }
    scene.configure_artwork(pages.clone());
    *effective = (pack.is_some() || session_icons.is_some()).then_some(pages);
    (session_icons, icon_locations)
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
    cape_state: Local<'s, crate::presentation::cape::CapeState>,
    skin_rigs: Local<'s, crate::presentation::skin_rig::SkinRigCache>,
    skin_pack: Local<'s, crate::presentation::actors::SkinLayerPack>,
    hand_builder: ResMut<'w, HandRigBuilder>,
    hand_scene: ResMut<'w, HandRigScene>,
    hand_revision: Local<'s, u64>,
    local_skin: Res<'w, crate::player_skin::LocalPlayerSkin>,
    swings: Option<ResMut<'w, SwingTracker>>,
    hand_motion: Option<Res<'w, crate::camera::FirstPersonHandMotion>>,
    equipment: Option<ResMut<'w, EquipmentRuntime>>,
    ui: Option<Res<'w, crate::ui_runtime::UiRuntime>>,
    collisions: Option<Res<'w, crate::movement::PhysicsCollisionRegistries>>,
    item_use: Option<Res<'w, crate::item_use::ItemUseRuntime>>,
    dropped_items: DroppedItemPublisher<'w, 's>,
    profiler: Option<Res<'w, render::RuntimeStageProfiler>>,
    partial_tick: ResMut<'w, ActorFramePartialTick>,
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
        mut cape_state,
        mut skin_rigs,
        mut skin_pack,
        mut hand_builder,
        mut hand_scene,
        mut hand_revision,
        local_skin,
        mut swings,
        hand_motion,
        mut equipment,
        collisions,
        item_use,
        ui,
        mut dropped_items,
        profiler,
        mut partial_tick,
    } = params;
    let _timer = profiler
        .as_deref()
        .map(|profiler| profiler.time(render::RuntimeStage::ActorPublication));
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
        if let Some(stream) = client_world.stream.as_mut() {
            stream.set_actor_seat_defaults(super::seat_defaults::seat_defaults());
        }
        let pack = session_id.and_then(|_| client_world.pack_entities.clone());
        let items = session_id.and_then(|_| client_world.session_items.clone());
        let staged = StagedSessionIcons::stage(items.as_deref());
        let (staged, locations) = if pack.is_some() || staged.is_some() || session_artwork.is_some()
        {
            apply_session_pack(
                &mut scene,
                &artwork,
                pack.as_deref(),
                staged,
                &mut session_artwork,
                equipment.as_deref_mut(),
            )
        } else {
            (None, Vec::new())
        };
        if let Some(equipment) = equipment.as_deref_mut() {
            equipment.set_session_items(items.as_deref(), staged, locations);
        }
    }
    let artwork = session_artwork.as_ref().unwrap_or(&artwork);
    let step = actor_clock.advance(time.delta());
    partial_tick.0 = step.partial_tick;
    skin_rigs.begin_frame();
    let first_person = settings.perspective() == semantic_input::PerspectiveMode::FirstPerson;
    let item_use = client_world
        .stream
        .as_ref()
        .zip(ui.as_deref())
        .zip(item_use.as_deref())
        .map_or(LocalItemUse::Unpredicted, |((stream, ui), item_use)| {
            item_use.local_item_use(stream, ui)
        });
    let mut local_feed = build_local_player_feed(
        &local_physics,
        view.rotation(),
        first_person,
        &local_skin,
        item_use,
    );
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
        // Attacks, mining and use swing the local arm at once; the server echoes no swing.
        if let Some(ticks) = swings.as_deref_mut().and_then(SwingTracker::take_started) {
            stream.start_local_player_swing(ticks);
        }
        let (yaw, pitch, _) = view.rotation().to_euler(bevy::math::EulerRot::YXZ);
        stream.set_actor_camera_rotation([
            -pitch.to_degrees(),
            (180.0 - yaw.to_degrees()).rem_euclid(360.0),
        ]);
        // Fluid and bed state is tick state; a frame without a tick would resample the same.
        if step.ticks > 0
            && let Some(collisions) = collisions.as_deref()
        {
            super::actor_sampling::sample_actor_world_state(stream, collisions);
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
    // Vanilla projects the hand with its own fixed FOV, ignoring the FOV option and modifiers.
    let hand_camera_fov = camera
        .single()
        .ok()
        .filter(|(_, projection)| matches!(projection, Projection::Perspective(_)))
        .map(|_| HAND_FOV_DEGREES.to_radians());
    // Registered together below: each registration rebuilds and re-uploads the whole catalog.
    let mut new_geometries = Vec::new();
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
                        actor_rig_presentation(&rig, actor, profile, step.partial_tick).map(
                            |mut presentation| {
                                if let Some(geometry) = rig.skin_geometry {
                                    // The pose drives the skin's own bones, so only its model fits.
                                    match skin_rigs.rig(geometry, |built| {
                                        if let Some(equipment) = equipment.as_deref_mut() {
                                            equipment.register_skin_rig(
                                                built.id,
                                                geometry
                                                    .bones
                                                    .iter()
                                                    .map(|bone| bone.name.clone())
                                                    .collect(),
                                            );
                                        }
                                        new_geometries.push(built);
                                    }) {
                                        Some(id) => presentation.submission.input.rig = id,
                                        None => {
                                            presentation.submission.route =
                                                render::ActorRigRoute::NoDraw;
                                        }
                                    }
                                }
                                presentation
                            },
                        )
                    } else {
                        crate::presentation::actors::entity_rig_presentation(
                            &rig,
                            actor,
                            artwork,
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
            let consume_ticks = ui
                .as_deref()
                .and_then(|ui| crate::item_use::consume_ticks(stream, ui));
            let hand = stream.actor_rig(local_runtime_id).map_or(
                FirstPersonHand {
                    swing: 0.0,
                    equip: 1.0,
                    consume: None,
                },
                |rig| hand_progress(rig.hand, consume_ticks, step.partial_tick),
            );
            let item = input.main.as_ref().and_then(|item| {
                let FirstPersonItem { layer, view_space } =
                    equipment.first_person_item(&presentation.submission, item, hand)?;
                let page = usize::from(layer.location.page()).checked_sub(1)?;
                let page = artwork.pages().get(page)?;
                let (width, height) = page.dimensions();
                let atlas = HandItemAtlas {
                    width,
                    height,
                    layers: page.layers(),
                    rgba8: page.shared_pixels(),
                };
                Some((layer, atlas, view_space))
            });
            // Provisional: vanilla draws every held item; an undrawable one shows the bare arm.
            let arms = FirstPersonArms::for_hands(
                input.main.as_ref().map(|item| item.identifier.as_ref()),
                input.off.as_ref().map(|item| item.identifier.as_ref()),
            )
            .with_undrawn_main(item.is_some());
            let body = equipment.mask_first_person(&presentation.submission, arms);
            (body.is_some() || item.is_some()).then_some(HandSource {
                presentation,
                body,
                item,
                motion: hand_motion
                    .as_deref()
                    .map_or(Mat4::IDENTITY, hand_motion_matrix),
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
    if let Some(stream) = client_world.stream.as_ref() {
        crate::presentation::entity_layers::apply_render_layers(
            &mut batch,
            |runtime_id| stream.actor_rig(runtime_id),
            artwork,
        );
    }
    if let (Some(stream), Some(cape)) = (
        client_world.stream.as_ref(),
        cape_state.rig(client_world.entity_assets.as_deref()),
    ) {
        if !scene.contains_geometry(cape.id) {
            let _ = scene.insert_geometry(cape.geometry.clone());
        }
        crate::presentation::cape::apply_capes(
            &mut batch,
            cape,
            |runtime_id| stream.actor_rig(runtime_id),
            |runtime_id| stream.actor_player_profile(runtime_id),
        );
    }
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
            if (identity.layer == render::ACTOR_LAYER_BODY
                || identity.layer == crate::presentation::cape::ACTOR_LAYER_CAPE
                || identity.layer >= crate::presentation::entity_layers::ACTOR_LAYER_TEXTURE_BASE)
                && stream
                    .actor(identity.runtime_id)
                    .is_some_and(|actor| actor.is_invisible())
            {
                submission.route = render::ActorRigRoute::NoDraw;
            }
        }
    }
    if let Some(equipment) = equipment.as_deref_mut() {
        new_geometries.extend(equipment.take_pending_geometries());
    }
    register_geometries(&mut hand_builder.0, &mut scene, new_geometries);
    *frame = update_actor_rig_scene(&mut scene, step.partial_tick, batch, &mut skin_pack).clone();
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

/// Registers new skin models and item meshes in both rig catalogs with one rebuild each; if a
/// batch is refused, each is tried alone so a rejected mesh only leaves that model undrawn.
fn register_geometries(
    hand: &mut ActorRigFrameBuilder,
    scene: &mut ActorRenderScene,
    geometries: Vec<render::ActorRigGeometry>,
) {
    if geometries.is_empty() {
        return;
    }
    if hand.insert_geometries(geometries.clone()).is_err() {
        for geometry in geometries.iter().cloned() {
            let _ = hand.insert_geometry(geometry);
        }
    }
    if scene.insert_geometries(geometries.clone()).is_err() {
        for geometry in geometries {
            let _ = scene.insert_geometry(geometry);
        }
    }
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
    let placement = hand_camera_from_rig(source.presentation.authored_scale, source.motion);
    let mut submissions = Vec::new();
    if let Some(mut body) = source.body {
        body.world_from_actor = placement;
        // The hand skin is a single-layer array; the third-person layer index does not apply.
        body.texture_layer = 0;
        submissions.push(body);
    }
    let mut atlas = None;
    if let Some((layer, item_atlas, view_space)) = source.item {
        let mut item = layer.submission;
        item.world_from_actor = if view_space {
            view_placement(source.motion)
        } else {
            placement
        };
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

/// Camera-space rows for a first-person item placed by its own view transform under `motion`.
fn view_placement(motion: Mat4) -> [[f32; 4]; 3] {
    let rows = motion.transpose().to_cols_array_2d();
    [rows[0], rows[1], rows[2]]
}

/// The arm's state at `partial_tick` between the rig's last two ticks, as `renderFirstPerson`
/// interpolates it: the swing wraps forward past its end, and an eat or drink use of
/// `consume_ticks` counts from its first using tick.
fn hand_progress(
    hand: [client_world::HandPhase; 2],
    consume_ticks: Option<u32>,
    partial_tick: f32,
) -> FirstPersonHand {
    let [previous, current] = hand;
    let mut swing = current.attack_time - previous.attack_time;
    if swing < 0.0 {
        swing += 1.0;
    }
    FirstPersonHand {
        swing: previous.attack_time + swing * partial_tick,
        equip: previous.arm_height + (current.arm_height - previous.arm_height) * partial_tick,
        consume: consume_ticks
            .filter(|_| current.use_ticks > 0)
            .map(|ticks| (current.use_ticks as f32 - 1.0 + partial_tick, ticks as f32)),
    }
}

/// What the first-person pass draws: arm-masked body pose and/or a held item with its atlas page
/// and whether its bone is in camera space.
struct HandSource {
    presentation: ActorRigPresentation,
    body: Option<ActorRigSubmission>,
    item: Option<(EquipmentPresentation, HandItemAtlas, bool)>,
    /// View-space hurt tilt, walk bob and sway applied before the rig placement.
    motion: Mat4,
}

/// Vanilla's hand stack order: hurt tilt, walk bob, then sway about X and Y.
fn hand_motion_matrix(motion: &crate::camera::FirstPersonHandMotion) -> Mat4 {
    motion.hurt
        * motion.bob.matrix()
        * Mat4::from_rotation_x(motion.sway_pitch_radians)
        * Mat4::from_rotation_y(motion.sway_yaw_radians)
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
    item_use: LocalItemUse,
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
        item_use,
    })
}

#[cfg(test)]
mod tests {
    use client_world::HandPhase;

    // The swing wraps forward from its last tick to rest, and an eat use counts from its first
    // using tick only while the rig reports the use.
    #[test]
    fn hand_progress_interpolates_the_swing_forward_and_counts_the_use() {
        let phase = |attack_time, arm_height, use_ticks| HandPhase {
            attack_time,
            arm_height,
            use_ticks,
        };
        let hand = super::hand_progress([phase(5.0 / 6.0, 0.6, 0), phase(0.0, 1.0, 0)], None, 0.5);
        assert!((hand.swing - 11.0 / 12.0).abs() < 1e-6);
        assert!((hand.equip - 0.8).abs() < 1e-6);
        assert_eq!(hand.consume, None);
        let eating = super::hand_progress([phase(0.0, 1.0, 3), phase(0.0, 1.0, 4)], Some(32), 0.25);
        assert_eq!(eating.consume, Some((3.25, 32.0)));
        let idle = super::hand_progress([phase(0.0, 1.0, 0), phase(0.0, 1.0, 0)], Some(32), 0.25);
        assert_eq!(idle.consume, None);
    }

    // The pack's first-person arm offset sits behind the model's left side; vanilla's facing puts
    // that ahead of the view and to its right.
    #[test]
    fn first_person_arm_offset_lands_ahead_and_right_of_the_camera() {
        let rows = super::hand_camera_from_rig(0.9375, bevy::math::Mat4::IDENTITY);
        let arm = [-8.5 / 16.0, 12.0 / 16.0, 12.0 / 16.0];
        let camera: [f32; 3] = std::array::from_fn(|row| {
            (0..3).map(|axis| rows[row][axis] * arm[axis]).sum::<f32>() + rows[row][3]
        });
        assert!(
            camera[0] > 0.0 && camera[1] < 0.0 && camera[2] < 0.0,
            "{camera:?}"
        );
    }
}
