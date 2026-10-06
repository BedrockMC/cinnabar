//! Java 1.7 player animation at publication: Java's pose, body yaw and first-person hand
//! replace vanilla's wherever Java has the posture.

use std::sync::Arc;

use bevy::math::{Mat4, Vec3};
use chunk_pipeline::WorldStream;
use client_world::{ActorRigSnapshot, ActorSnapshot, BoneTransform};
use render_model::{
    RenderBoneTransform,
    java_animation::{self as java, JavaBiped, JavaBipedInput, JavaHand, JavaUse, is_java_sword},
};

use super::hand::{HandSource, hand_progress, item_atlas};
use crate::presentation::{
    actors::ActorRigPresentation,
    equipment::{
        ActorEquipmentInput, EquipmentRuntime, FirstPersonArms, WornItem, java_draws_attachable,
        remote_input,
    },
};

const BOW: &str = "minecraft:bow";
const FILLED_MAP: &str = "minecraft:filled_map";
/// Pose bones Java's parts drive, in [`JavaBiped::parts`] order.
const PART_BONES: [&str; 6] = ["head", "body", "rightarm", "leftarm", "rightleg", "leftleg"];
const RIGHT_ARM: usize = 2;
/// Other players draw this much lower while sneaking.
const REMOTE_SNEAK_DROP: f32 = 0.125;
/// Java lifts the model this many pixels above the feet.
const MODEL_LIFT_PIXELS: f32 = 0.125;

/// Java's use of the main-hand `item` at the frame, from the use flag's tick count.
pub(super) fn java_use(
    item: &str,
    use_ticks: u32,
    consume_ticks: Option<u32>,
    alpha: f32,
) -> Option<JavaUse> {
    if use_ticks == 0 {
        return None;
    }
    let ticks = use_ticks as f32;
    if is_java_sword(item) {
        Some(JavaUse::Block)
    } else if item == BOW {
        Some(JavaUse::Bow {
            pull: ticks - 2.0 + alpha,
        })
    } else {
        consume_ticks.map(|duration| JavaUse::Consume {
            remaining: duration as f32 - ticks + 2.0 - alpha,
            duration: duration as f32,
        })
    }
}

/// Java's bow frame by whole draw ticks: standby, then its three pull frames.
pub(super) fn java_bow_frame(use_ticks: u32) -> u32 {
    match use_ticks.saturating_sub(1) {
        0 => 0,
        1..=13 => 1,
        14..=17 => 2,
        _ => 3,
    }
}

/// Java's first-person hand at the frame.
pub(super) fn first_person_hand(
    rig: &ActorRigSnapshot<'_>,
    main: Option<&str>,
    consume_ticks: Option<u32>,
    alpha: f32,
) -> JavaHand {
    let [previous, current] = rig.java.equip;
    JavaHand {
        swing: hand_progress(rig.hand, None, alpha).swing,
        equip: previous + (current - previous) * alpha,
        using: main.and_then(|item| java_use(item, rig.hand[1].use_ticks, consume_ticks, alpha)),
    }
}

/// Model-space targets for `parts` of `pose` on this rig, by pose index; `None` when the rig
/// lacks one of the parts.
fn targets(
    rig: &ActorRigSnapshot<'_>,
    pose: &JavaBiped,
    parts: &[usize],
) -> Option<Vec<Option<BoneTransform>>> {
    let mut targets = vec![None; rig.rest.len()];
    let all = pose.parts();
    for &part in parts {
        let index = rig
            .bone_names
            .iter()
            .position(|name| **name == *PART_BONES[part])?;
        let rest = rig.rest.get(index)?;
        let (posed, rest_part) = all[part];
        let (rotation, translation) =
            posed.rig_bone(rest_part, Vec3::from_slice(&rest.translation_scale[..3]));
        *targets.get_mut(index)? = Some(BoneTransform {
            rotation: rotation.to_array(),
            translation_scale: [translation.x, translation.y, translation.z, 1.0],
            axis_scale: [1.0; 3],
        });
    }
    Some(targets)
}

fn render_bones(pose: &[BoneTransform]) -> Option<Arc<[RenderBoneTransform]>> {
    pose.iter()
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

fn retargeted(
    stream: &WorldStream,
    rig: &ActorRigSnapshot<'_>,
    pose: &JavaBiped,
    parts: &[usize],
    alpha: f32,
) -> Option<Arc<[RenderBoneTransform]>> {
    let targets = targets(rig, pose, parts)?;
    render_bones(&stream.authority().actor_retargeted_pose(
        rig.actor.runtime_id,
        alpha,
        &targets,
    )?)
}

fn lerp_degrees(from: f32, to: f32, alpha: f32) -> f32 {
    from + wrap_degrees(to - from) * alpha
}

fn wrap_degrees(degrees: f32) -> f32 {
    (degrees + 180.0).rem_euclid(360.0) - 180.0
}

/// The rig with Java's body yaw and Java's pose at the frame, unless vanilla keeps this
/// player's posture; `local` holds the client's own equipment.
pub(super) fn third_person<'a>(
    stream: &WorldStream,
    rig: &ActorRigSnapshot<'a>,
    actor: &ActorSnapshot,
    local: Option<&ActorEquipmentInput>,
    alpha: f32,
) -> Option<(ActorRigSnapshot<'a>, Arc<[RenderBoneTransform]>)> {
    if rig.java.vanilla_posture {
        return None;
    }
    let java_rig = ActorRigSnapshot {
        previous_body_yaw: rig.java.body_yaw[0],
        body_yaw: rig.java.body_yaw[1],
        ..*rig
    };
    let main = match local {
        Some(equipment) => equipment.main.clone(),
        None => remote_input(stream, actor.runtime_id).main,
    }
    .map(|item| item.identifier);
    let bones = third_person_pose(stream, &java_rig, actor, main.as_deref(), alpha)?;
    Some((java_rig, bones))
}

/// Replaces the presentation's pose with Java's and lifts it as Java draws players.
pub(super) fn apply_pose(
    presentation: &mut ActorRigPresentation,
    bones: &Arc<[RenderBoneTransform]>,
    local: bool,
    actor: &ActorSnapshot,
) {
    let submission = &mut presentation.submission;
    submission.input.previous_bones = Arc::clone(bones);
    submission.input.current_bones = Arc::clone(bones);
    // The local body is placed at its render feet afterwards, which then lifts it.
    if !local {
        lift(&mut submission.world_from_actor, actor.is_sneaking());
    }
}

/// Java's third-person pose at the frame for a player holding `main_hand`.
fn third_person_pose(
    stream: &WorldStream,
    rig: &ActorRigSnapshot<'_>,
    actor: &ActorSnapshot,
    main_hand: Option<&str>,
    alpha: f32,
) -> Option<Arc<[RenderBoneTransform]>> {
    let motion = rig.java;
    let lerp = |[from, to]: [f32; 2]| from + (to - from) * alpha;
    let body_yaw = lerp_degrees(motion.body_yaw[0], motion.body_yaw[1], alpha);
    let head_yaw = lerp_degrees(actor.previous_pose.head_yaw, actor.head_yaw, alpha);
    let using = actor.is_using_item();
    let input = JavaBipedInput {
        limb_swing: motion.limb_swing[1] - motion.limb_amount[1] * (1.0 - alpha),
        limb_amount: lerp(motion.limb_amount).min(1.0),
        age: actor.status.age_ticks as f32 + alpha,
        head_yaw: wrap_degrees(head_yaw - body_yaw),
        head_pitch: lerp([actor.previous_pose.pitch, actor.pitch]),
        swing: hand_progress(rig.hand, None, alpha).swing,
        sneaking: actor.is_sneaking(),
        riding: motion.riding,
        held_right: match main_hand {
            None => 0,
            Some(item) if using && is_java_sword(item) => 3,
            Some(_) => 1,
        },
        aimed_bow: using && main_hand == Some(BOW),
    };
    retargeted(stream, rig, &java::java_biped(&input), &[0, 1, 2, 3, 4, 5], alpha)
}

/// Java's lift above the feet, and the sneaking drop other players get.
pub(super) fn lift(world_from_actor: &mut [[f32; 4]; 3], sneaking_remote: bool) {
    for row in world_from_actor.iter_mut() {
        row[3] += row[1] * MODEL_LIFT_PIXELS / 16.0;
    }
    if sneaking_remote {
        world_from_actor[1][3] -= REMOTE_SNEAK_DROP;
    }
}

/// The empty hand's arm bones in Java's rest pose, and camera space from the rig frame.
pub(super) fn first_person_arm(
    stream: &WorldStream,
    rig: &ActorRigSnapshot<'_>,
    hand: JavaHand,
    alpha: f32,
) -> Option<(Arc<[RenderBoneTransform]>, Mat4)> {
    let pose = java::java_biped(&JavaBipedInput::default());
    let bones = retargeted(stream, rig, &pose, &[RIGHT_ARM], alpha)?;
    Some((bones, java::first_person_arm(hand.swing, hand.equip)))
}

/// What Java's first-person hand reads this frame.
pub(super) struct HandInputs<'a> {
    pub(super) stream: &'a WorldStream,
    pub(super) presentation: ActorRigPresentation,
    pub(super) equipment_input: &'a ActorEquipmentInput,
    pub(super) consume_ticks: Option<u32>,
    pub(super) item_animation: Option<client_world::AttachableAnimationInput<'static>>,
    pub(super) alpha: f32,
    pub(super) artwork: &'a render::ActorArtworkPages,
    pub(super) motion: Mat4,
}

/// Java's first-person hand: the item it still draws through an equip dip, or its empty arm.
/// `None` leaves vanilla's hand for items Java never had (maps, crossbows, shields).
pub(super) fn hand_source(
    inputs: HandInputs<'_>,
    equipment: &mut EquipmentRuntime,
    shown: &mut Option<WornItem>,
) -> Option<HandSource> {
    let HandInputs {
        stream,
        presentation,
        equipment_input,
        consume_ticks,
        item_animation,
        alpha,
        artwork,
        motion,
    } = inputs;
    let runtime_id = presentation.submission.input.identity.runtime_id;
    let rig = stream.authority().actor_rig(runtime_id)?;
    let main = rig.java_equipped.and_then(|equipped| match &equipment_input.main {
        Some(item) if item.identifier == *equipped => {
            *shown = Some(item.clone());
            Some(item.clone())
        }
        _ => shown
            .clone()
            .filter(|old| old.identifier == *equipped)
            .or_else(|| equipment_input.main.clone()),
    });
    if let Some(item) = &main
        && (&*item.identifier == FILLED_MAP || equipment.is_vanilla_attachable(&item.identifier))
    {
        return None;
    }
    let hand = first_person_hand(
        &rig,
        main.as_ref().map(|item| item.identifier.as_ref()),
        consume_ticks,
        alpha,
    );
    let body_pose = &presentation.submission;
    let main_layer = main.as_ref().and_then(|item| {
        let attachable = java_draws_attachable(&item.identifier).then(|| {
            let mut input = item_animation?;
            input.frame_alpha = alpha;
            input.animation_frame = java_bow_frame(rig.hand[1].use_ticks);
            let input = equipment_input.attachable_input(input.for_hand(false));
            let actor = stream.authority().actor(runtime_id)?;
            equipment.first_person_attachable(body_pose, item, actor, &rig, input, Some(hand))
        });
        let layer = attachable
            .flatten()
            .or_else(|| equipment.first_person_java_item(body_pose, item, hand))?;
        let atlas = item_atlas(&layer, artwork)?;
        Some((layer, atlas))
    });
    let off_layer = equipment_input.off.as_ref().and_then(|item| {
        let layer = equipment.first_person_offhand(body_pose, item)?;
        let atlas = item_atlas(&layer, artwork)?;
        Some((layer, atlas))
    });
    // Java draws the arm only with an empty hand; an undrawable item shows it too.
    let arm = main_layer.is_none().then(|| first_person_arm(stream, &rig, hand, alpha));
    let (body, java_body_camera) = match arm.flatten() {
        Some((bones, camera)) => {
            let mut posed = presentation.submission.clone();
            posed.input.previous_bones = Arc::clone(&bones);
            posed.input.current_bones = bones;
            let arms = FirstPersonArms {
                right: true,
                left: false,
            };
            (equipment.mask_first_person(&posed, arms), Some(camera))
        }
        None => (None, None),
    };
    Some(HandSource {
        presentation,
        body,
        items: [main_layer, off_layer],
        motion,
        java_body_camera,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Draw and eat timing count from the first using tick, a tick behind the frame.
    #[test]
    fn use_timing_maps_java_counts() {
        assert_eq!(java_use("minecraft:apple", 0, Some(32), 0.5), None);
        assert_eq!(
            java_use("minecraft:bow", 1, None, 0.25),
            Some(JavaUse::Bow { pull: -0.75 })
        );
        assert_eq!(
            java_use("minecraft:apple", 3, Some(32), 0.5),
            Some(JavaUse::Consume {
                remaining: 30.5,
                duration: 32.0
            })
        );
        assert_eq!(
            java_use("minecraft:iron_sword", 1, None, 0.0),
            Some(JavaUse::Block)
        );
        assert_eq!(java_use("minecraft:stick", 4, None, 0.0), None);
    }

    #[test]
    fn bow_frames_follow_java_draw_thresholds() {
        let frames = [0, 1, 2, 14, 15, 18, 19, 40].map(java_bow_frame);
        assert_eq!(frames, [0, 0, 1, 1, 2, 2, 3, 3]);
    }

    #[test]
    fn third_person_lift_and_sneak_drop() {
        let mut rows = [[-1.0, 0.0, 0.0, 5.0], [0.0, 0.9375, 0.0, 64.0], [0.0, 0.0, -1.0, 2.0]];
        lift(&mut rows, true);
        assert!((rows[1][3] - (64.0 + 0.9375 / 128.0 - 0.125)).abs() < 1e-6);
        assert_eq!(rows[0][3], 5.0);
    }
}
