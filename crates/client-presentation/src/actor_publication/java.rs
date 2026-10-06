//! Java 1.7 player animation at publication: Java's pose, body yaw and first-person hand
//! replace vanilla's wherever Java has the posture.

use std::sync::Arc;

use bevy::math::{Mat4, Vec3};
use chunk_pipeline::WorldStream;
use client_world::{ActorRigSnapshot, ActorSnapshot, BoneTransform, JavaHeldItem, SkinRenderLayer};
use render_model::{
    RenderBoneTransform,
    java_animation::{
        self as java, JavaBiped, JavaBipedInput, JavaCapeInput, JavaHand, JavaUse, is_java_sword,
    },
};

use super::hand::{HandSource, item_atlas};
use crate::presentation::{
    actors::{ActorRigPresentation, convert_bones, lerp_degrees, wrap_degrees},
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
const REMOTE_SNEAK_DROP: f32 = 0.125;
const LOCAL_SNEAK_DROP: f32 = 0.2 * 0.4;
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
        swing: rig.java.swing_progress(alpha),
        equip: previous + (current - previous) * alpha,
        using: main.and_then(|item| java_use(item, rig.hand[1].use_ticks, consume_ticks, alpha)),
    }
}

/// Model-space targets for `parts` of `pose` on a skeleton, by pose index. A `partial`
/// skeleton (a skin layer) skips parts it lacks; otherwise a missing part yields `None`.
fn targets(
    names: &[Box<str>],
    rest: &[BoneTransform],
    pose: &JavaBiped,
    parts: &[usize],
    partial: bool,
) -> Option<Vec<Option<BoneTransform>>> {
    let mut targets = vec![None; rest.len()];
    let all = pose.parts();
    for &part in parts {
        let found = names
            .iter()
            .position(|name| name.eq_ignore_ascii_case(PART_BONES[part]));
        let index = match found {
            Some(index) => index,
            None if partial => continue,
            None => return None,
        };
        let (posed, rest_part) = all[part];
        let pivot = Vec3::from_slice(&rest.get(index)?.translation_scale[..3]);
        let (rotation, translation) = posed.rig_bone(rest_part, pivot);
        *targets.get_mut(index)? = Some(BoneTransform {
            rotation: rotation.to_array(),
            translation_scale: [translation.x, translation.y, translation.z, 1.0],
            axis_scale: [1.0; 3],
        });
    }
    Some(targets)
}

fn retargeted(
    stream: &WorldStream,
    rig: &ActorRigSnapshot<'_>,
    pose: &JavaBiped,
    parts: &[usize],
    alpha: f32,
) -> Option<Arc<[RenderBoneTransform]>> {
    let targets = targets(rig.bone_names, rig.rest, pose, parts, false)?;
    convert_bones(&stream.authority().actor_retargeted_pose(
        rig.actor.runtime_id,
        alpha,
        &targets,
    )?)
}

/// Java's body-yaw rig, pose and animated skin layers for a player at the frame, unless
/// vanilla keeps its posture; `local` holds the client's own equipment.
pub(super) struct ThirdPerson<'a> {
    pub(super) rig: ActorRigSnapshot<'a>,
    pub(super) bones: Arc<[RenderBoneTransform]>,
    pub(super) posed: Posed,
}

/// What later layers of a player Java posed this frame read.
pub(super) struct Posed {
    runtime_id: u64,
    pub(super) skin_layers: Vec<SkinRenderLayer>,
    pub(super) cape: JavaCapeInput,
}

pub(super) fn third_person<'a>(
    stream: &WorldStream,
    rig: &ActorRigSnapshot<'a>,
    actor: &ActorSnapshot,
    local: Option<&ActorEquipmentInput>,
    alpha: f32,
) -> Option<ThirdPerson<'a>> {
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
    let pose = java::java_biped(&third_person_input(
        &java_rig,
        actor,
        main.as_deref(),
        alpha,
    ));
    let parts = [0, 1, 2, 3, 4, 5];
    let bones = retargeted(stream, &java_rig, &pose, &parts, alpha)?;
    let skin_layers = if rig.skin_layers.is_empty() {
        Vec::new()
    } else {
        stream
            .authority()
            .actor_retargeted_layers(actor.runtime_id, alpha, |names, rest| {
                targets(names, rest, &pose, &parts, true)
            })?
    };
    let motion = rig.java;
    let lerp = |[from, to]: [f32; 2]| from + (to - from) * alpha;
    let [chase_from, chase_to] = motion.cape.map(Vec3::from_array);
    let cape = JavaCapeInput {
        chase: chase_from.lerp(chase_to, alpha),
        body_yaw: lerp_degrees(motion.body_yaw[0], motion.body_yaw[1], alpha),
        bob: lerp(motion.bob),
        walked: lerp(motion.walked),
        sneaking: actor.is_sneaking(),
    };
    Some(ThirdPerson {
        rig: java_rig,
        bones,
        posed: Posed {
            runtime_id: actor.runtime_id,
            skin_layers,
            cape,
        },
    })
}

/// The player Java posed this frame, if it was.
pub(super) fn posed(posed: &[Posed], runtime_id: u64) -> Option<&Posed> {
    posed.iter().find(|posed| posed.runtime_id == runtime_id)
}

/// Replaces the presentation's pose with Java's and lifts it as Java draws players.
pub(super) fn apply_pose(
    presentation: &mut ActorRigPresentation,
    bones: &Arc<[RenderBoneTransform]>,
    local: bool,
    actor: &ActorSnapshot,
    alpha: f32,
) {
    let submission = &mut presentation.submission;
    if let Some(progress) = actor.death_rotation_progress(alpha) {
        submission.world_from_actor = death_tilt(
            submission.world_from_actor,
            progress,
            f32::from(actor.status.death_time) + alpha,
        );
    }
    submission.input.previous_bones = Arc::clone(bones);
    submission.input.current_bones = Arc::clone(bones);
    // The local body is placed at its render feet afterwards, which then lifts it.
    if !local {
        lift(&mut submission.world_from_actor, actor.is_sneaking(), false);
    }
}

/// Replaces the native body's death angle with Java's faster sideways fall.
fn death_tilt(mut rows: [[f32; 4]; 3], native_progress: f32, death_ticks: f32) -> [[f32; 4]; 3] {
    let java_progress =
        ((death_ticks - 1.0) / f32::from(client_world::DEATH_DURATION_TICKS) * 1.6).clamp(0.0, 1.0);
    let angle = (java_progress.sqrt() - native_progress.clamp(0.0, 1.0).sqrt())
        * std::f32::consts::FRAC_PI_2;
    let (sine, cosine) = angle.sin_cos();
    for row in &mut rows {
        let (x, y) = (row[0], row[1]);
        row[0] = x * cosine + y * sine;
        row[1] = -x * sine + y * cosine;
    }
    rows
}

/// Java's pose inputs at the frame for a player holding `main_hand`.
fn third_person_input(
    rig: &ActorRigSnapshot<'_>,
    actor: &ActorSnapshot,
    main_hand: Option<&str>,
    alpha: f32,
) -> JavaBipedInput {
    let motion = rig.java;
    let lerp = |[from, to]: [f32; 2]| from + (to - from) * alpha;
    let body_yaw = lerp_degrees(motion.body_yaw[0], motion.body_yaw[1], alpha);
    let head_yaw = lerp_degrees(actor.previous_pose.head_yaw, actor.head_yaw, alpha);
    let using = actor.is_using_item();
    JavaBipedInput {
        limb_swing: motion.limb_swing[1] - motion.limb_amount[1] * (1.0 - alpha),
        limb_amount: lerp(motion.limb_amount).min(1.0),
        age: actor.status.age_ticks as f32 + alpha,
        head_yaw: wrap_degrees(head_yaw - body_yaw),
        head_pitch: lerp([actor.previous_pose.pitch, actor.pitch]),
        swing: rig.java.swing_progress(alpha),
        sneaking: actor.is_sneaking(),
        riding: motion.riding,
        held_right: match main_hand {
            None => 0,
            Some(item) if using && is_java_sword(item) => 3,
            Some(_) => 1,
        },
        aimed_bow: using && main_hand == Some(BOW),
    }
}

/// Java's lift above the feet, less its sneaking drop: other players draw 0.125 lower, the
/// local player by its eased 0.2 · 0.4 step offset.
pub(super) fn lift(world_from_actor: &mut [[f32; 4]; 3], sneaking: bool, local: bool) {
    for row in world_from_actor.iter_mut() {
        row[3] += row[1] * MODEL_LIFT_PIXELS / 16.0;
    }
    if sneaking {
        world_from_actor[1][3] -= if local {
            LOCAL_SNEAK_DROP
        } else {
            REMOTE_SNEAK_DROP
        };
    }
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

type ArmKey = (client_world::ActorLifetimeId, u32, u64);

/// Frame-to-frame state of Java's first-person hand.
#[derive(Default)]
pub(super) struct HandCache {
    /// The main-hand item still drawn through an equip dip.
    shown: Option<WornItem>,
    /// The empty hand's rest pose, by actor lifetime, rig and rest generation.
    arm: Option<(ArmKey, Arc<[RenderBoneTransform]>)>,
}

/// Matches the item and data value retained by the equip animation.
fn is_held(item: &WornItem, held: &JavaHeldItem) -> bool {
    item.identifier == held.identifier && item.damage.unwrap_or(item.metadata) == held.metadata
}

impl HandCache {
    /// Remembers the held item once Java's equip adopts it, in any perspective.
    pub(super) fn remember(&mut self, rig: &ActorRigSnapshot<'_>, main: Option<&WornItem>) {
        if let (Some(item), Some(equipped)) = (main, rig.java_equipped)
            && is_held(item, equipped)
        {
            self.shown = Some(item.clone());
        }
    }

    /// The main-hand stack the first-person hand shows: the one Java's equip still holds
    /// through a dip, which becomes `selected` at its bottom.
    pub(super) fn displayed_main(
        &self,
        equipped: Option<&JavaHeldItem>,
        selected: Option<&WornItem>,
    ) -> Option<WornItem> {
        let equipped = equipped?;
        match selected {
            Some(item) if is_held(item, equipped) => Some(item.clone()),
            _ => self
                .shown
                .clone()
                .filter(|old| is_held(old, equipped))
                .or_else(|| selected.cloned()),
        }
    }
}

/// Vanilla's own hand draws a shown item Java never had (crossbows, shields, maps in either
/// hand); deciding on the shown item swaps hands at the bottom of the dip, where both are lowest.
fn vanilla_draws(
    main: Option<&WornItem>,
    off: Option<&WornItem>,
    vanilla_attachable: impl Fn(&str) -> bool,
) -> bool {
    let map = |item: &WornItem| &*item.identifier == FILLED_MAP;
    main.is_some_and(|item| map(item) || vanilla_attachable(&item.identifier))
        || off.is_some_and(map)
}

/// Java's first-person hand for `equipment_input`, whose main hand is the displayed stack:
/// the item it still draws through an equip dip, or its empty arm. `None` leaves vanilla's hand.
pub(super) fn hand_source(
    inputs: HandInputs<'_>,
    equipment: &mut EquipmentRuntime,
    cache: &mut HandCache,
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
    let main = equipment_input.main.clone();
    if vanilla_draws(main.as_ref(), equipment_input.off.as_ref(), |identifier| {
        equipment.is_vanilla_attachable(identifier)
    }) {
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
        let attachable = item_animation.and_then(|mut input| {
            input.frame_alpha = alpha;
            let input = equipment_input.attachable_input(input.for_hand(true));
            let actor = stream.authority().actor(runtime_id)?;
            equipment.first_person_attachable(body_pose, item, actor, &rig, input, None)
        });
        let layer = attachable.or_else(|| equipment.first_person_offhand(body_pose, item))?;
        let atlas = item_atlas(&layer, artwork)?;
        Some((layer, atlas))
    });
    // Java draws the arm only with an empty hand; an undrawable item shows it too. The pose is
    // fixed, so it is retargeted once per rig.
    let key = (rig.actor, rig.rig.0, rig.rest_reset_generation);
    let arm = main_layer.is_none().then(|| {
        let bones = match &cache.arm {
            Some((cached, bones)) if *cached == key => Arc::clone(bones),
            _ => {
                let pose = java::java_biped(&JavaBipedInput::default());
                let bones = retargeted(stream, &rig, &pose, &[RIGHT_ARM], alpha)?;
                cache.arm = Some((key, Arc::clone(&bones)));
                bones
            }
        };
        Some((bones, java::first_person_arm(hand.swing, hand.equip)))
    });
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

    /// A head-only skin layer takes the head target and skips the parts it lacks.
    #[test]
    fn partial_layers_skip_missing_parts() {
        let names = [Box::from("head")];
        let rest = [BoneTransform {
            rotation: [0.0, 0.0, 0.0, 1.0],
            translation_scale: [0.0, 24.0, 0.0, 1.0],
            axis_scale: [1.0; 3],
        }];
        let pose = java::java_biped(&JavaBipedInput::default());
        let parts = [0, 1, 2, 3, 4, 5];
        assert!(targets(&names, &rest, &pose, &parts, true).unwrap()[0].is_some());
        assert!(targets(&names, &rest, &pose, &parts, false).is_none());
    }

    /// Builds a sprite stack for the hand transition tests.
    fn worn(identifier: &str) -> WornItem {
        WornItem {
            identifier: Arc::from(identifier),
            metadata: 0,
            damage: None,
            kind: crate::presentation::equipment::HeldKind::Sprite,
            dye_rgb: None,
        }
    }

    /// Extracts the identity that the equip animation retains.
    fn held(item: &WornItem) -> JavaHeldItem {
        JavaHeldItem {
            identifier: Arc::clone(&item.identifier),
            metadata: item.damage.unwrap_or(item.metadata),
            stack_id: None,
        }
    }

    /// Between a Java item and one only vanilla draws, the old item stays in its own hand
    /// until the dip's bottom, in both directions.
    #[test]
    fn mixed_swaps_change_hands_at_the_bottom_of_the_dip() {
        const CROSSBOW: &str = "minecraft:crossbow";
        const SHIELD: &str = "minecraft:shield";
        let vanilla = |identifier: &str| matches!(identifier, CROSSBOW | SHIELD);
        let id = |item: Option<WornItem>| item.map(|item| item.identifier);
        for (old, new) in [
            ("minecraft:iron_sword", CROSSBOW),
            (CROSSBOW, "minecraft:iron_sword"),
            ("minecraft:iron_sword", SHIELD),
            (SHIELD, "minecraft:iron_sword"),
        ] {
            let (old, new) = (worn(old), worn(new));
            let cache = HandCache {
                shown: Some(old.clone()),
                ..HandCache::default()
            };
            let dipping = cache.displayed_main(Some(&held(&old)), Some(&new));
            assert_eq!(
                vanilla_draws(dipping.as_ref(), None, vanilla),
                vanilla(&old.identifier)
            );
            assert_eq!(id(dipping), Some(Arc::clone(&old.identifier)));
            let adopted = cache.displayed_main(Some(&held(&new)), Some(&new));
            assert_eq!(
                vanilla_draws(adopted.as_ref(), None, vanilla),
                vanilla(&new.identifier)
            );
            assert_eq!(id(adopted), Some(Arc::clone(&new.identifier)));
        }
    }

    /// Two data values of one item are different stacks to the dip.
    #[test]
    fn displayed_stack_keeps_its_data_value_through_the_dip() {
        let water = worn("minecraft:potion");
        let healing = WornItem {
            metadata: 21,
            ..worn("minecraft:potion")
        };
        let cache = HandCache {
            shown: Some(water.clone()),
            ..HandCache::default()
        };
        let shown = cache.displayed_main(Some(&held(&water)), Some(&healing));
        assert_eq!(shown.map(|item| item.metadata), Some(0));
    }

    /// Java's body reaches its side during the fourteenth tick, while the native curve is still falling.
    #[test]
    fn death_body_reaches_java_angle_without_moving_its_feet() {
        use crate::presentation::actors::death_tilted;
        let base = [
            [1.0, 0.0, 0.0, 3.0],
            [0.0, 1.0, 0.0, 64.0],
            [0.0, 0.0, 1.0, 5.0],
        ];
        for ticks in [1.0_f32, 5.5, 13.5, 20.0] {
            let native = ticks / f32::from(client_world::DEATH_DURATION_TICKS);
            let rows = death_tilt(death_tilted(base, Some(native)), native, ticks);
            let angle = (((ticks - 1.0) / 20.0 * 1.6).sqrt().min(1.0) * 90.0).to_radians();
            assert!((rows[0][0] - angle.cos()).abs() < 1e-6);
            assert!((rows[1][0] - angle.sin()).abs() < 1e-6);
            assert_eq!([rows[0][3], rows[1][3], rows[2][3]], [3.0, 64.0, 5.0]);
        }
    }

    #[test]
    fn third_person_lift_and_sneak_drop() {
        let mut rows = [
            [-1.0, 0.0, 0.0, 5.0],
            [0.0, 0.9375, 0.0, 64.0],
            [0.0, 0.0, -1.0, 2.0],
        ];
        lift(&mut rows, true, false);
        assert!((rows[1][3] - (64.0 + 0.9375 / 128.0 - 0.125)).abs() < 1e-6);
        assert_eq!(rows[0][3], 5.0);
    }
}
