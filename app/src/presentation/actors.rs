use std::{collections::BTreeMap, sync::Arc};

use assets::EntityRigFallback;
use client_world::{ActorRigSnapshot, ActorSnapshot, PlayerProfile};
use protocol::{ActorKind, PlayerSkin};
use render::{
    ActorArtworkLocation, ActorArtworkPages, ActorCullView, ActorRenderFrame, ActorRenderIdentity,
    ActorRenderScene, ActorRigRenderInput, ActorRigRoute, ActorRigSubmission, ActorSkinPixels,
    EntityRigId, MAX_RENDERED_PLAYERS, RenderBoneTransform, actor_rig_submission_is_visible,
    default_actor_skin_rgba8, normalize_actor_skin,
};

#[derive(Clone, Debug)]
pub(crate) struct ActorRigPresentation {
    pub(crate) submission: ActorRigSubmission,
    pub(crate) skin_rgba8: Option<Arc<[u8]>>,
    pub(crate) artwork: Option<ActorArtworkLocation>,
    pub(crate) model_scale: f32,
    /// Head yaw minus the rendered body yaw, in degrees.
    pub(crate) head_over_body: f32,
}

#[derive(Debug)]
pub(crate) struct ActorPresentationBatch {
    pub(crate) submissions: Vec<ActorRigSubmission>,
    pub(crate) skins_rgba8: Arc<[u8]>,
    pub(crate) artwork: BTreeMap<ActorRenderIdentity, ActorArtworkLocation>,
}

pub(crate) fn update_actor_rig_scene(
    scene: &mut ActorRenderScene,
    partial_tick: f32,
    batch: ActorPresentationBatch,
) -> &ActorRenderFrame {
    // The app adapter has already applied the renderer's exact culling helper
    // to remotes before enforcing capacity. Passing no second cull view keeps
    // Phase 3's visible local reservation unconditional in both third-person
    // modes while the render-owned builder still validates every other field.
    scene.update_rigs_with_artwork(
        partial_tick,
        None,
        batch.submissions,
        batch.skins_rgba8,
        &batch.artwork,
    )
}

pub(crate) fn entity_rig_presentation(
    rig: &ActorRigSnapshot<'_>,
    actor: &ActorSnapshot,
    artwork: &ActorArtworkPages,
    partial_tick: f32,
) -> Option<ActorRigPresentation> {
    let location = matches!(actor.kind, ActorKind::Entity { .. })
        .then(|| artwork.route(EntityRigId(rig.rig.0)))
        .flatten();
    let rest_mode =
        location.is_some_and(|location| location.pose_mode() == assets::ActorPoseMode::RestPose);
    let bad_rest = rest_mode
        && (rig.rest.is_empty()
            || rig.rest.len() != rig.previous.len()
            || rig.rest.len() != rig.current.len()
            || !rig.rest.iter().all(|bone| {
                RenderBoneTransform::from_model_space_scaled(
                    bone.rotation,
                    bone.translation_scale,
                    bone.axis_scale,
                )
                .is_some()
            }));
    let selected = if rest_mode {
        ActorRigSnapshot {
            previous: rig.rest,
            current: rig.rest,
            completed_tick: rig.rest_completed_tick,
            reset_generation: rig.rest_reset_generation,
            ..*rig
        }
    } else {
        *rig
    };
    let mut presentation =
        actor_rig_presentation_inner(&selected, actor, None, partial_tick, bad_rest)?;
    if matches!(actor.kind, ActorKind::Entity { .. })
        && let Some(location) = location
    {
        presentation.submission.route = match rig.fallback {
            EntityRigFallback::Skip => ActorRigRoute::Compiled,
            EntityRigFallback::GeometryOnly => ActorRigRoute::StaticFallback,
            EntityRigFallback::Diagnostic => ActorRigRoute::NoDraw,
        };
        if rest_mode {
            presentation.submission.route =
                if bad_rest || rig.fallback == EntityRigFallback::Diagnostic {
                    ActorRigRoute::NoDraw
                } else {
                    ActorRigRoute::StaticFallback
                };
        }
        presentation.submission.texture_layer = location.layer();
        presentation.artwork = Some(location);
    }
    Some(presentation)
}

pub(crate) fn actor_rig_presentation(
    rig: &ActorRigSnapshot<'_>,
    actor: &ActorSnapshot,
    profile: Option<&PlayerProfile>,
    partial_tick: f32,
) -> Option<ActorRigPresentation> {
    actor_rig_presentation_inner(rig, actor, profile, partial_tick, false)
}

fn actor_rig_presentation_inner(
    rig: &ActorRigSnapshot<'_>,
    actor: &ActorSnapshot,
    profile: Option<&PlayerProfile>,
    partial_tick: f32,
    rejected_pose: bool,
) -> Option<ActorRigPresentation> {
    if rig.actor.runtime_id != actor.runtime_id
        || rig.actor.spawn_revision != actor.spawn_revision
        || rig.actor.session_id == 0
        || rig.actor.runtime_id == 0
        || rig.actor.spawn_revision == 0
        || rig.completed_tick == 0
        || rig.reset_generation == 0
        || (!rejected_pose && (rig.previous.is_empty() || rig.previous.len() != rig.current.len()))
        || !partial_tick.is_finite()
    {
        return None;
    }

    // A rejected submission retains exact ownership for observable NoDraw counts,
    // but contains no substitute pose and can never reach a GPU draw.
    let previous_bones = if rejected_pose {
        Arc::from([])
    } else {
        convert_bones(rig.previous)?
    };
    let current_bones = if rejected_pose {
        Arc::from([])
    } else {
        convert_bones(rig.current)?
    };
    let alpha = partial_tick.clamp(0.0, 1.0);
    let position = interpolated_position(actor, alpha)?;
    let yaw = lerp_degrees(rig.previous_body_yaw, rig.body_yaw, alpha);
    if !yaw.is_finite() || !rig.scale.is_finite() || rig.scale <= 0.0 {
        return None;
    }
    let identity = ActorRenderIdentity {
        session_id: rig.actor.session_id,
        dimension: rig.actor.dimension,
        runtime_id: rig.actor.runtime_id,
        spawn_revision: rig.actor.spawn_revision,
        ingress_sequence: actor.spawn_revision.max(actor.movement_revision),
        source_tick: actor.source_tick,
        movement_revision: actor.movement_revision,
        pose_generation: rig.completed_tick,
    };
    if !identity.is_exact() {
        return None;
    }

    let (route, skin_rgba8) = player_route_and_skin(actor, profile, rig.fallback);
    Some(ActorRigPresentation {
        submission: ActorRigSubmission {
            input: ActorRigRenderInput {
                identity,
                rig: EntityRigId(rig.rig.0),
                previous_bones,
                current_bones,
                completed_tick: rig.completed_tick,
                reset_generation: rig.reset_generation,
            },
            world_from_actor: rig_world_from_actor(position, yaw, rig.scale),
            texture_layer: u32::MAX,
            route,
        },
        skin_rgba8,
        artwork: None,
        model_scale: rig.scale,
        head_over_body: wrap_degrees(
            lerp_degrees(actor.previous_pose.head_yaw, actor.head_yaw, alpha) - yaw,
        ),
    })
}

pub(crate) fn local_diagnostic_presentation(
    actor_session_id: u64,
    dimension: i32,
    runtime_id: u64,
    pose_generation: u64,
    position: [f32; 3],
    yaw_degrees: f32,
    pitch_degrees: f32,
) -> Option<ActorRigPresentation> {
    if actor_session_id == 0
        || runtime_id == 0
        || pose_generation == 0
        || position.iter().any(|value| !value.is_finite())
        || !yaw_degrees.is_finite()
        || !pitch_degrees.is_finite()
    {
        return None;
    }
    let head_rotation = quaternion_from_euler_degrees([pitch_degrees, 0.0, 0.0]);
    let pivots = [
        [0.0, 1.75, 0.0],
        [0.0, 1.5, 0.0],
        [-0.3125, 1.375, 0.0],
        [-0.11875, 0.75, 0.0],
        [0.3125, 1.375, 0.0],
        [0.11875, 0.75, 0.0],
    ];
    let mut bones = pivots.map(|pivot| RenderBoneTransform {
        rotation: [0.0, 0.0, 0.0, 1.0],
        translation_scale: [pivot[0], pivot[1], pivot[2], 1.0],
        axis_scale: render::UNIT_AXIS_SCALE,
    });
    bones[0].rotation = head_rotation;
    Some(ActorRigPresentation {
        submission: ActorRigSubmission {
            input: ActorRigRenderInput {
                identity: ActorRenderIdentity {
                    session_id: actor_session_id,
                    dimension,
                    runtime_id,
                    spawn_revision: actor_session_id,
                    ingress_sequence: pose_generation,
                    source_tick: None,
                    movement_revision: pose_generation,
                    pose_generation,
                },
                rig: EntityRigId(u32::MAX),
                previous_bones: Arc::from(bones),
                current_bones: Arc::from(bones),
                completed_tick: pose_generation,
                reset_generation: actor_session_id,
            },
            // Same facing convention as the driven rig so the pre-rig fallback and the rig agree.
            world_from_actor: rig_world_from_actor(position, yaw_degrees, 1.0),
            texture_layer: u32::MAX,
            route: ActorRigRoute::Diagnostic,
        },
        skin_rgba8: Some(default_actor_skin_rgba8()),
        artwork: None,
        model_scale: 1.0,
        head_over_body: 0.0,
    })
}

pub(crate) fn local_actor_presentation_for_visibility(
    local_runtime_id: u64,
    visibility_runtime_id: u64,
    canonical: Option<ActorRigPresentation>,
    diagnostic: Option<ActorRigPresentation>,
    yaw_degrees: f32,
) -> Option<ActorRigPresentation> {
    if local_runtime_id == 0 || visibility_runtime_id != local_runtime_id {
        return None;
    }
    let diagnostic = diagnostic.filter(|presentation| {
        presentation.submission.input.identity.runtime_id == local_runtime_id
    })?;
    match canonical {
        Some(mut canonical)
            if canonical.submission.input.identity.runtime_id == local_runtime_id =>
        {
            // The body lags the view yaw as the rig's head does, so the head faces the view.
            let feet = diagnostic.submission.world_from_actor.map(|row| row[3]);
            canonical.submission.world_from_actor = rig_world_from_actor(
                feet,
                yaw_degrees - canonical.head_over_body,
                canonical.model_scale,
            );
            Some(canonical)
        }
        Some(_) => None,
        None => Some(diagnostic),
    }
}

#[cfg(test)]
pub(crate) fn select_actor_presentations(
    local_runtime_id: u64,
    local_visible: bool,
    local: Option<ActorRigPresentation>,
    remotes: impl IntoIterator<Item = ActorRigPresentation>,
) -> ActorPresentationBatch {
    select_actor_presentations_for_view(local_runtime_id, local_visible, local, remotes, None)
}

pub(crate) fn select_actor_presentations_for_view(
    local_runtime_id: u64,
    local_visible: bool,
    local: Option<ActorRigPresentation>,
    remotes: impl IntoIterator<Item = ActorRigPresentation>,
    view: Option<ActorCullView>,
) -> ActorPresentationBatch {
    let mut latest = BTreeMap::<u64, ActorRigPresentation>::new();
    for remote in remotes {
        let identity = remote.submission.input.identity;
        if identity.runtime_id == 0 || identity.runtime_id == local_runtime_id {
            continue;
        }
        match latest.entry(identity.runtime_id) {
            std::collections::btree_map::Entry::Vacant(entry) => {
                entry.insert(remote);
            }
            std::collections::btree_map::Entry::Occupied(mut entry) => {
                if identity > entry.get().submission.input.identity {
                    entry.insert(remote);
                }
            }
        }
    }

    let local = local_visible
        .then_some(local)
        .flatten()
        .filter(|local| local.submission.input.identity.runtime_id == local_runtime_id);
    let mut selected = Vec::with_capacity(MAX_RENDERED_PLAYERS);
    let mut drawable_count = 0usize;
    if let Some(local) = local {
        drawable_count = 1;
        selected.push(local);
    }
    for remote in latest.into_values() {
        if remote.submission.route == ActorRigRoute::NoDraw {
            selected.push(remote);
            continue;
        }
        if drawable_count == MAX_RENDERED_PLAYERS
            || !actor_rig_submission_is_visible(&remote.submission, view)
        {
            continue;
        }
        drawable_count += 1;
        selected.push(remote);
    }

    let mut artwork = BTreeMap::new();
    let mut skin_families = Vec::<Arc<[u8]>>::new();
    let mut submissions = Vec::with_capacity(selected.len());
    for mut presentation in selected {
        if let Some(location) = presentation.artwork {
            artwork.insert(presentation.submission.input.identity, location);
            submissions.push(presentation.submission);
            continue;
        }
        let Some(skin) = presentation.skin_rgba8 else {
            presentation.submission.route = ActorRigRoute::NoDraw;
            presentation.submission.texture_layer = u32::MAX;
            submissions.push(presentation.submission);
            continue;
        };
        let layer = skin_families
            .iter()
            .position(|existing| existing.as_ref() == skin.as_ref())
            .unwrap_or_else(|| {
                skin_families.push(skin);
                skin_families.len() - 1
            });
        presentation.submission.texture_layer =
            u32::try_from(layer).expect("actor skin family count is bounded");
        submissions.push(presentation.submission);
    }
    let mut skin_bytes = Vec::new();
    for skin in skin_families {
        skin_bytes.extend_from_slice(&skin);
    }
    ActorPresentationBatch {
        submissions,
        skins_rgba8: skin_bytes.into(),
        artwork,
    }
}

fn convert_bones(bones: &[client_world::BoneTransform]) -> Option<Arc<[RenderBoneTransform]>> {
    bones
        .iter()
        .map(|bone| {
            RenderBoneTransform::from_model_space_scaled(
                bone.rotation,
                bone.translation_scale,
                bone.axis_scale,
            )
        })
        .collect::<Option<Vec<_>>>()
        .map(Arc::from)
}

/// Places a rig-frame model, which faces -Z with its right side at +X, so it faces the
/// Minecraft `yaw_degrees` direction at `position`, scaled about the feet.
pub(crate) fn rig_world_from_actor(
    position: [f32; 3],
    yaw_degrees: f32,
    scale: f32,
) -> [[f32; 4]; 3] {
    let (sine, cosine) = yaw_degrees.to_radians().sin_cos();
    [
        [-cosine * scale, 0.0, sine * scale, position[0]],
        [0.0, scale, 0.0, position[1]],
        [-sine * scale, 0.0, -cosine * scale, position[2]],
    ]
}

fn interpolated_position(actor: &ActorSnapshot, partial_tick: f32) -> Option<[f32; 3]> {
    let position = std::array::from_fn(|axis| {
        actor.previous_pose.position[axis]
            + (actor.position[axis] - actor.previous_pose.position[axis]) * partial_tick
    });
    position
        .iter()
        .all(|value| value.is_finite())
        .then_some(position)
}

fn lerp_degrees(start: f32, end: f32, alpha: f32) -> f32 {
    wrap_degrees(start + wrap_degrees(end - start) * alpha)
}

fn wrap_degrees(degrees: f32) -> f32 {
    (degrees + 180.0).rem_euclid(360.0) - 180.0
}

fn quaternion_from_euler_degrees(rotation: [f32; 3]) -> [f32; 4] {
    let [x, y, z] = rotation.map(|value| value.to_radians() * 0.5);
    let (sx, cx) = x.sin_cos();
    let (sy, cy) = y.sin_cos();
    let (sz, cz) = z.sin_cos();
    [
        sx * cy * cz - cx * sy * sz,
        cx * sy * cz + sx * cy * sz,
        cx * cy * sz - sx * sy * cz,
        cx * cy * cz + sx * sy * sz,
    ]
}

fn player_route_and_skin(
    actor: &ActorSnapshot,
    profile: Option<&PlayerProfile>,
    fallback: EntityRigFallback,
) -> (ActorRigRoute, Option<Arc<[u8]>>) {
    let ActorKind::Player { .. } = &actor.kind else {
        return (ActorRigRoute::NoDraw, None);
    };
    let route = match fallback {
        EntityRigFallback::Skip => ActorRigRoute::Compiled,
        EntityRigFallback::GeometryOnly => ActorRigRoute::StaticFallback,
        EntityRigFallback::Diagnostic => ActorRigRoute::Diagnostic,
    };
    let skin = profile
        .filter(|profile| profile.unique_id == actor.unique_id)
        .and_then(|profile| match &profile.skin {
            PlayerSkin::Standard(skin) => normalize_actor_skin(&ActorSkinPixels {
                width: skin.width,
                height: skin.height,
                rgba8: Arc::clone(&skin.rgba8),
            }),
            PlayerSkin::Unavailable(_) => None,
        })
        .unwrap_or_else(default_actor_skin_rgba8);
    (route, Some(skin))
}
