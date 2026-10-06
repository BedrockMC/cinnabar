//! First-person hand frame publication, including independent main/offhand artwork.

use super::*;
use crate::presentation::equipment::ActorEquipmentInput;

/// Vanilla draws the first-person rig in view space as a zero-yaw actor, feet one eye height
/// below the camera; the pack's first-person arm offsets are authored for that facing.
pub(super) fn hand_camera_from_rig(scale: f32, motion: Mat4) -> [[f32; 4]; 3] {
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
    // The vanilla first-person actor root retains its 1/128-model-unit lift
    // after the player model scale.
    camera_space_rows(motion * placement * Mat4::from_translation(bevy::math::Vec3::Y / 128.0))
}

pub(super) fn camera_space_rows(matrix: Mat4) -> [[f32; 4]; 3] {
    let composed = matrix.transpose().to_cols_array_2d();
    [composed[0], composed[1], composed[2]]
}

/// Builds and publishes the local player's first-person rig for the near-camera pass, or clears
/// it when not in first person, when the look FOV is unavailable, or when no skin resolved.
pub(super) fn publish_hand_rig(
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
        body.world_from_actor = source.java_body_camera.map_or(placement, |camera| {
            camera_space_rows(source.motion * camera)
        });
        // The hand skin is a single-layer array; the third-person layer index does not apply.
        body.texture_layer = 0;
        submissions.push(body);
    }
    let mut atlases = [None, None];
    for (index, entry) in source.items.into_iter().enumerate() {
        if let Some((layer, item_atlas)) = entry {
            let mut item = layer.presentation.submission;
            item.world_from_actor = match layer.java_camera {
                Some(camera) => camera_space_rows(source.motion * camera),
                None if layer.camera_space => camera_space_rows(source.motion),
                None => placement,
            };
            item.texture_layer = layer.presentation.location.layer()
                | render::HAND_ITEM_LAYER_FLAG
                | layer.alpha_mode.texture_layer_flag()
                | if index == 1 {
                    render::HAND_OFFHAND_LAYER_FLAG
                } else {
                    0
                };
            submissions.push(item);
            atlases[index] = Some(item_atlas);
        }
    }
    let frame = builder.build(partial_tick, None, submissions);
    *revision = revision.wrapping_add(1).max(1);
    if scene.publish(frame, skin, light, fov, *revision) {
        scene.set_item_atlases(atlases);
    }
}

/// The arm's state at `partial_tick` between the rig's last two ticks, as vanilla's first-person
/// pass interpolates it: the swing wraps forward past its end, and an eat or drink use of
/// `consume_ticks` counts from its first using tick.
pub(super) fn hand_progress(
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
pub(super) struct HandSource {
    pub(super) presentation: ActorRigPresentation,
    pub(super) body: Option<ActorRigSubmission>,
    pub(super) items: [Option<(FirstPersonItem, HandItemAtlas)>; 2],
    /// View-space hurt tilt, walk bob and sway applied before the rig placement.
    pub(super) motion: Mat4,
    /// Camera from the body's rig frame under Java's empty-hand stack.
    pub(super) java_body_camera: Option<Mat4>,
}

/// Inputs shared by the built-in hand stacks and authored attachables.
pub(super) struct HandInputs<'a> {
    pub(super) stream: &'a WorldStream,
    pub(super) presentation: ActorRigPresentation,
    pub(super) equipment_input: &'a ActorEquipmentInput,
    pub(super) owner_equipment: &'a ActorEquipmentInput,
    pub(super) consume_ticks: Option<u32>,
    pub(super) item_animation: Option<client_world::AttachableAnimationInput<'static>>,
    pub(super) alpha: f32,
    pub(super) artwork: &'a render::ActorArtworkPages,
    pub(super) motion: Mat4,
}

pub(super) fn vanilla_hand_source(
    inputs: HandInputs<'_>,
    equipment: &mut EquipmentRuntime,
    hand: FirstPersonHand,
) -> Option<HandSource> {
    let HandInputs {
        stream,
        presentation,
        equipment_input,
        owner_equipment,
        item_animation,
        alpha,
        artwork,
        motion,
        ..
    } = inputs;
    let runtime_id = presentation.submission.input.identity.runtime_id;
    let rig = stream.authority().actor_rig(runtime_id)?;
    let items = std::array::from_fn(|index| {
        let item = [equipment_input.main.as_ref(), equipment_input.off.as_ref()][index]?;
        let modern = item_animation.and_then(|mut render_input| {
            render_input.frame_alpha = alpha;
            let render_input =
                attachable_hand_input(equipment_input, owner_equipment, render_input, index == 1);
            equipment.first_person_attachable(
                &presentation.submission,
                item,
                stream.authority().actor(runtime_id)?,
                &rig,
                render_input,
                None,
            )
        });
        let layer = modern.or_else(|| {
            if index == 0 {
                equipment.first_person_item(&presentation.submission, item, hand)
            } else {
                equipment.first_person_offhand(&presentation.submission, item)
            }
        })?;
        let atlas = item_atlas(&layer, artwork)?;
        Some((layer, atlas))
    });
    let arms = FirstPersonArms::for_hands(
        equipment_input
            .main
            .as_ref()
            .map(|item| item.identifier.as_ref()),
        equipment_input
            .off
            .as_ref()
            .map(|item| item.identifier.as_ref()),
    )
    .with_undrawn_main(items[0].is_some());
    let body = equipment.mask_first_person(&presentation.submission, arms);
    (body.is_some() || items.iter().any(Option::is_some)).then_some(HandSource {
        presentation,
        body,
        items,
        motion,
        java_body_camera: None,
    })
}

/// An outgoing main draw stays idle in its retained item frame; the offhand reads the actual
/// owner's equipment and main-hand use timing.
pub(super) fn attachable_hand_input<'a>(
    rendered: &'a ActorEquipmentInput,
    owner: &'a ActorEquipmentInput,
    timing: client_world::AttachableAnimationInput<'a>,
    off_hand: bool,
) -> client_world::AttachableAnimationInput<'a> {
    let selected = rendered.main.as_ref().map(|item| &item.identifier)
        == owner.main.as_ref().map(|item| &item.identifier);
    let timing = if off_hand || selected {
        timing
    } else {
        client_world::AttachableAnimationInput {
            animation_frame: 0,
            use_elapsed_ticks: None,
            hand_charged: false,
            ..timing
        }
    };
    let equipment = if off_hand { owner } else { rendered };
    equipment.attachable_input(timing.for_hand(off_hand))
}

/// The artwork page an item layer samples, as the hand pass binds it.
pub(super) fn item_atlas(
    layer: &FirstPersonItem,
    artwork: &render::ActorArtworkPages,
) -> Option<HandItemAtlas> {
    let page = usize::from(layer.presentation.location.page()).checked_sub(1)?;
    let page = artwork.pages().get(page)?;
    let (width, height) = page.dimensions();
    Some(HandItemAtlas {
        width,
        height,
        layers: page.layers(),
        rgba8: page.shared_pixels(),
    })
}

/// Vanilla's hand stack order: hurt tilt, walk bob, then sway about X and Y.
pub(super) fn hand_motion_matrix(motion: &crate::camera::FirstPersonHandMotion) -> Mat4 {
    motion.hurt
        * motion.bob.matrix()
        * Mat4::from_rotation_x(motion.sway_pitch_radians)
        * Mat4::from_rotation_y(motion.sway_yaw_radians)
}
