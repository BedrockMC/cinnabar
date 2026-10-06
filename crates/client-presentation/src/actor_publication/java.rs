//! Java 1.7 player animation at publication: Java's pose, body yaw and first-person hand
//! replace vanilla's wherever Java has the posture.

use std::sync::Arc;

use bevy::math::{Mat4, Vec3};
use chunk_pipeline::WorldStream;
use client_world::{ActorRigSnapshot, ActorSnapshot, BoneTransform, SkinRenderLayer};
use render_model::{
    RenderBoneTransform,
    java_animation::{
        self as java, JavaBiped, JavaBipedInput, JavaCapeInput, JavaHand, JavaUse, is_java_sword,
    },
};

use super::hand::{HandInputs, HandSource, hand_progress, item_atlas, vanilla_hand_source};
use crate::presentation::{
    actors::{ActorRigPresentation, convert_bones, lerp_degrees, wrap_degrees},
    equipment::{
        ActorEquipmentInput, EquipmentRuntime, FirstPersonArms, FirstPersonHand, WornItem,
        java_draws_attachable, remote_input,
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
    selected: Option<&str>,
    use_ticks: u32,
    consume_ticks: Option<u32>,
    alpha: f32,
) -> Option<JavaUse> {
    if selected != Some(item) || use_ticks == 0 {
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
    selected: Option<&str>,
    consume_ticks: Option<u32>,
    alpha: f32,
) -> JavaHand {
    let [previous, current] = rig.java.equip;
    JavaHand {
        swing: hand_progress(rig.hand, None, alpha).swing,
        equip: previous + (current - previous) * alpha,
        using: main
            .and_then(|item| java_use(item, selected, rig.hand[1].use_ticks, consume_ticks, alpha)),
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
    if rig.java.vanilla_posture
        || render_model::is_pack_rig_id(render_model::EntityRigId(rig.rig.0))
    {
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
        local.is_some(),
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
) {
    let submission = &mut presentation.submission;
    submission.input.previous_bones = Arc::clone(bones);
    submission.input.current_bones = Arc::clone(bones);
    // The local body is placed at its render feet afterwards, which then lifts it.
    if !local {
        lift(&mut submission.world_from_actor, actor.is_sneaking(), false);
    }
}

/// Java's pose inputs at the frame for a player holding `main_hand`.
fn third_person_input(
    rig: &ActorRigSnapshot<'_>,
    actor: &ActorSnapshot,
    main_hand: Option<&str>,
    alpha: f32,
    local: bool,
) -> JavaBipedInput {
    let motion = rig.java;
    let lerp = |[from, to]: [f32; 2]| from + (to - from) * alpha;
    let body_yaw = lerp_degrees(motion.body_yaw[0], motion.body_yaw[1], alpha);
    // Local look arrives every frame; current actor angles advance only at fixed ticks.
    let (head_yaw, head_pitch) = if local {
        (actor.received_pose.head_yaw, actor.received_pose.pitch)
    } else {
        (
            lerp_degrees(actor.previous_pose.head_yaw, actor.head_yaw, alpha),
            lerp([actor.previous_pose.pitch, actor.pitch]),
        )
    };
    let using = actor.is_using_item();
    JavaBipedInput {
        limb_swing: motion.limb_swing[1] - motion.limb_amount[1] * (1.0 - alpha),
        limb_amount: lerp(motion.limb_amount).min(1.0),
        age: actor.status.age_ticks as f32 + alpha,
        head_yaw: wrap_degrees(head_yaw - body_yaw),
        head_pitch,
        swing: hand_progress(rig.hand, None, alpha).swing,
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

type ArmKey = (client_world::ActorLifetimeId, u32, u64);

/// Frame-to-frame state of Java's first-person hand.
#[derive(Default)]
pub(super) struct HandCache {
    /// The main-hand item still drawn through an equip dip.
    shown: Option<WornItem>,
    /// The empty hand's rest pose, by actor lifetime, rig and rest generation.
    arm: Option<(ArmKey, Arc<[RenderBoneTransform]>)>,
}

impl HandCache {
    /// Remembers the held item once Java's equip adopts it, in any perspective.
    pub(super) fn remember(&mut self, rig: &ActorRigSnapshot<'_>, main: Option<&WornItem>) {
        if let (Some(item), Some(equipped)) = (main, rig.java_equipped)
            && item.identifier == *equipped
        {
            self.shown = Some(item.clone());
        }
    }
}

fn retained_item(
    equipped: Option<&str>,
    selected: Option<&WornItem>,
    remembered: Option<&WornItem>,
) -> Option<WornItem> {
    let equipped = equipped?;
    selected
        .filter(|item| item.identifier.as_ref() == equipped)
        .or_else(|| remembered.filter(|item| item.identifier.as_ref() == equipped))
        .cloned()
}

/// Java's first-person hand: the item it still draws through an equip dip, or its empty arm.
/// Items Java never had keep their authored placement but adopt at the bottom of Java's dip.
/// An authored player rig keeps ownership of its whole first-person hand.
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
        ..
    } = inputs;
    let runtime_id = presentation.submission.input.identity.runtime_id;
    let rig = stream.authority().actor_rig(runtime_id)?;
    if render_model::is_pack_rig_id(render_model::EntityRigId(rig.rig.0)) {
        return None;
    }
    let main = retained_item(
        rig.java_equipped.map(|item| item.as_ref()),
        equipment_input.main.as_ref(),
        cache.shown.as_ref(),
    );
    let selected = equipment_input
        .main
        .as_ref()
        .map(|item| item.identifier.as_ref());
    let retained = main.as_ref().map(|item| item.identifier.as_ref());
    let hand = first_person_hand(&rig, retained, selected, consume_ticks, alpha);
    let map = |item: &WornItem| &*item.identifier == FILLED_MAP;
    let vanilla_only =
        |item: &&WornItem| map(item) || equipment.is_vanilla_attachable(&item.identifier);
    if main.as_ref().is_some_and(|item| vanilla_only(&item))
        || equipment_input.off.as_ref().is_some_and(map)
    {
        let retained_equipment = ActorEquipmentInput {
            main: main.clone(),
            ..equipment_input.clone()
        };
        let progress = FirstPersonHand {
            swing: hand.swing,
            equip: hand.equip,
            consume: hand_progress(
                rig.hand,
                consume_ticks.filter(|_| retained == selected),
                alpha,
            )
            .consume,
        };
        return vanilla_hand_source(
            HandInputs {
                stream,
                presentation,
                equipment_input: &retained_equipment,
                owner_equipment: equipment_input,
                consume_ticks,
                item_animation,
                alpha,
                artwork,
                motion,
            },
            equipment,
            progress,
        );
    }
    let body_pose = &presentation.submission;
    let main_layer = main.as_ref().and_then(|item| {
        let attachable =
            (retained == selected && java_draws_attachable(&item.identifier)).then(|| {
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

    fn head_assets() -> Arc<assets::RuntimeEntityAssets> {
        let entity = br#"{"format_version":"1.10.0","minecraft:client_entity":{"description":{"identifier":"minecraft:player","materials":{"default":"entity"},"textures":{"default":"textures/entity/test"},"geometry":{"default":"geometry.test"},"render_controllers":["controller.render.test"]}}}"#;
        let geometry = br#"{"format_version":"1.12.0","minecraft:geometry":[{"description":{"identifier":"geometry.test","texture_width":64,"texture_height":64},"bones":[{"name":"head","pivot":[0,24,0]},{"name":"body","pivot":[0,24,0]},{"name":"rightarm","pivot":[5,22,0]},{"name":"leftarm","pivot":[-5,22,0]},{"name":"rightleg","pivot":[1.9,12,0]},{"name":"leftleg","pivot":[-1.9,12,0]}]}]}"#;
        let controller = br#"{"format_version":"1.8.0","render_controllers":{"controller.render.test":{"geometry":"Geometry.default","materials":[{"*":"Material.default"}],"textures":["Texture.default"]}}}"#;
        let compiled = pack_compiler::compile_entity_pack(vec![
            ("entity/player.json".into(), entity.to_vec()),
            ("models/entity/test.geo.json".into(), geometry.to_vec()),
            ("render_controllers/test.json".into(), controller.to_vec()),
        ])
        .unwrap()
        .unwrap();
        Arc::new(assets::RuntimeEntityAssets::from_compiled(compiled.assets).unwrap())
    }

    fn head_stream() -> WorldStream {
        WorldStream::new_with_asset_sets(
            protocol::WorldBootstrap {
                dimension: 0,
                local_player_runtime_id: 1,
                local_player_unique_id: 1,
                player_position: [0.0, 64.0, 0.0],
                world_spawn_position: [0, 64, 0],
                air_network_id: protocol::SEQUENTIAL_AIR_NETWORK_ID,
                block_network_ids_are_hashes: false,
            },
            Arc::new(assets::RuntimeAssets::diagnostic()),
            head_assets(),
            [0.0, 64.0, 0.0],
            None,
        )
    }

    fn head_feed() -> client_world::LocalPlayerFeed {
        client_world::LocalPlayerFeed {
            uuid: [1; 16],
            username: Arc::from("Player"),
            skin: protocol::PlayerSkin::Unavailable(
                protocol::PlayerSkinUnavailable::InvalidDimensions,
            ),
            position: [0.0, 64.0, 0.0],
            velocity: [0.0; 3],
            on_ground: true,
            yaw: 170.0,
            head_yaw: 170.0,
            pitch: 5.0,
            main_hand: None,
            off_hand: None,
            teleported: false,
            first_person: false,
            view_bobbing: true,
            sneaking: false,
            sprinting: false,
            item_use: client_world::LocalItemUse::Unpredicted,
        }
    }

    fn uploaded_skin_feed(animated: bool) -> client_world::LocalPlayerFeed {
        let mut feed = head_feed();
        let geometry = r#"{"format_version":"1.12.0","minecraft:geometry":[{"description":{"identifier":"geometry.uploaded","texture_width":64,"texture_height":64},"bones":[{"name":"head","pivot":[0,24,0]},{"name":"body","pivot":[0,24,0]},{"name":"rightarm","pivot":[4,22,0]},{"name":"leftarm","pivot":[-4,22,0]},{"name":"rightleg","pivot":[1.9,12,0]},{"name":"leftleg","pivot":[-1.9,12,0]}]}]}"#;
        let image = protocol::SkinAnimation {
            kind: protocol::SkinAnimationKind::Face,
            width: 64,
            height: 64,
            rgba8: vec![255; 64 * 64 * 4].into(),
            frames: 1,
            blinking: false,
        };
        feed.skin = protocol::PlayerSkin::Standard(protocol::StandardSkin {
            width: 64,
            height: 64,
            rgba8: vec![255; 64 * 64 * 4].into(),
            cape: None,
            geometry: Some(Arc::new(protocol::SkinGeometrySource {
                resource_patch: Arc::from(
                    r#"{"geometry":{"default":"geometry.uploaded","animated_face":"geometry.uploaded"}}"#,
                ),
                geometry_data: Arc::from(geometry),
                animations: if animated {
                    Arc::from([image])
                } else {
                    Arc::from([])
                },
            })),
        });
        feed
    }

    #[test]
    fn third_person_keeps_authored_pack_poses_and_accepts_uploaded_skin_geometry() {
        let mut stream = head_stream();
        stream.sync_local_player_pose(&uploaded_skin_feed(false));
        stream.advance_actor_interpolation_frame(1);
        let rig = stream.authority().actor_rig(1).unwrap();
        let actor = stream.authority().actor(1).unwrap();
        let equipment = ActorEquipmentInput::default();
        assert!(
            rig.skin_geometry.is_some(),
            "the uploaded model must be parsed"
        );
        assert!(!render_model::is_pack_rig_id(render_model::EntityRigId(
            rig.rig.0
        )));
        assert!(third_person(&stream, &rig, actor, Some(&equipment), 0.5).is_some());
        let authored = ActorRigSnapshot {
            rig: client_world::EntityRigId(assets::PACK_RIG_ID_BASE),
            ..rig
        };
        assert!(third_person(&stream, &authored, actor, Some(&equipment), 0.5).is_none());
    }

    #[test]
    fn published_persona_layers_follow_the_sampled_local_emote_only() {
        let mut stream = head_stream();
        stream.sync_local_player_pose(&uploaded_skin_feed(true));
        stream.advance_actor_interpolation_frame(1);
        let rig = stream.authority().actor_rig(1).unwrap();
        assert_eq!(rig.skin_layers.len(), 1);
        let native_layer = &rig.skin_layers[0];
        let native_pose = native_layer.current.clone();
        let emote = client_world::sample_custom_emote(
            &rig,
            client_world::CustomEmote::Twerk,
            0.0,
            client_world::CustomEmote::Twerk.duration_seconds() / 4.0,
        )
        .unwrap();
        assert_ne!(emote.skin_layers[0].current, native_pose);
        let published = super::super::emote_geometry::skin_layer_snapshot(rig, Some(&emote), None);
        assert_eq!(published.current, emote.current.as_ref());
        assert_eq!(
            published.skin_layers[0].current,
            emote.skin_layers[0].current
        );
        assert_eq!(
            published.skin_layers[0].previous,
            emote.skin_layers[0].previous
        );
        assert_eq!(published.skin_layers[0].image, native_layer.image);
        assert_eq!(published.skin_layers[0].uv_anim, native_layer.uv_anim);
        assert_eq!(native_layer.current, native_pose);
        let unchanged = super::super::emote_geometry::skin_layer_snapshot(rig, None, None);
        assert_eq!(unchanged.skin_layers[0].current, native_pose);
    }

    #[test]
    fn local_head_tracks_between_tick_look_while_moving_without_committing_a_pose() {
        let mut stream = head_stream();
        let mut feed = head_feed();
        stream.sync_local_player_pose(&feed);
        stream.advance_actor_interpolation_frame(1);
        feed.position[0] += 0.2;
        feed.velocity[0] = 0.2;
        feed.head_yaw = -170.0;
        feed.yaw = -170.0;
        stream.sync_local_player_pose(&feed);
        stream.advance_actor_interpolation_frame(1);
        let rig = stream.authority().actor_rig(1).unwrap();
        let original_motion = rig.java;
        let original_tick = rig.completed_tick;
        let original_pose = rig.current.to_vec();
        let tick_head = stream.authority().actor(1).unwrap().head_yaw;
        for (pitch, head_yaw) in [(20.0, -165.0), (-30.0, 179.0), (40.0, -179.0)] {
            feed.pitch = pitch;
            feed.head_yaw = head_yaw;
            feed.yaw = head_yaw;
            stream.sync_local_player_pose(&feed);
            stream.advance_actor_interpolation_frame(0);
            let actor = stream.authority().actor(1).unwrap();
            let original_actor = actor.clone();
            let rig = stream.authority().actor_rig(1).unwrap();
            for alpha in [0.0, 0.25, 0.75, 1.0] {
                let input = third_person_input(&rig, actor, None, alpha, true);
                let body = lerp_degrees(rig.java.body_yaw[0], rig.java.body_yaw[1], alpha);
                assert_eq!(input.head_pitch, pitch);
                assert!(wrap_degrees(input.head_yaw + body - head_yaw).abs() < 1e-4);
                let posed = java::java_biped(&input);
                let expected = java::java_biped(&JavaBipedInput {
                    head_pitch: pitch,
                    head_yaw: wrap_degrees(head_yaw - body),
                    ..input
                });
                assert_eq!(posed.head, expected.head);
            }
            assert_eq!(*actor, original_actor);
            assert_eq!(actor.head_yaw, tick_head);
            assert_eq!(actor.pitch, 5.0);
            assert_eq!(rig.java, original_motion);
            assert_eq!(rig.completed_tick, original_tick);
            assert_eq!(rig.current, original_pose.as_slice());
        }
    }

    #[test]
    fn nonlocal_head_keeps_tick_interpolation_and_takes_the_short_yaw_path() {
        let mut stream = head_stream();
        stream.sync_local_player_pose(&head_feed());
        stream.advance_actor_interpolation_frame(1);
        let mut actor = stream.authority().actor(1).unwrap().clone();
        actor.previous_pose.head_yaw = 179.0;
        actor.head_yaw = -179.0;
        actor.previous_pose.pitch = 10.0;
        actor.pitch = 30.0;
        actor.received_pose.head_yaw = 25.0;
        actor.received_pose.pitch = -40.0;
        let rig = stream.authority().actor_rig(1).unwrap();
        for alpha in [0.0, 0.25, 0.75, 1.0] {
            let input = third_person_input(&rig, &actor, None, alpha, false);
            let body = lerp_degrees(rig.java.body_yaw[0], rig.java.body_yaw[1], alpha);
            assert_eq!(input.head_pitch, 10.0 + 20.0 * alpha);
            assert!(wrap_degrees(input.head_yaw + body - (179.0 + 2.0 * alpha)).abs() < 1e-4);
        }
    }

    /// Draw and eat timing count from the first using tick, a tick behind the frame.
    #[test]
    fn use_timing_maps_java_counts() {
        assert_eq!(
            java_use("minecraft:apple", Some("minecraft:apple"), 0, Some(32), 0.5),
            None
        );
        assert_eq!(
            java_use("minecraft:bow", Some("minecraft:bow"), 1, None, 0.25),
            Some(JavaUse::Bow { pull: -0.75 })
        );
        assert_eq!(
            java_use("minecraft:apple", Some("minecraft:apple"), 3, Some(32), 0.5),
            Some(JavaUse::Consume {
                remaining: 30.5,
                duration: 32.0
            })
        );
        assert_eq!(
            java_use(
                "minecraft:iron_sword",
                Some("minecraft:iron_sword"),
                1,
                None,
                0.0
            ),
            Some(JavaUse::Block)
        );
        assert_eq!(
            java_use("minecraft:stick", Some("minecraft:stick"), 4, None, 0.0),
            None
        );
    }

    #[test]
    fn retained_item_does_not_use_the_selected_items_clock() {
        for (retained, selected, consume) in [
            ("minecraft:bow", Some("minecraft:apple"), Some(32)),
            ("minecraft:apple", Some("minecraft:bow"), None),
            ("minecraft:iron_sword", Some("minecraft:apple"), Some(32)),
            ("minecraft:bow", None, None),
        ] {
            assert_eq!(java_use(retained, selected, 4, consume, 0.5), None);
        }
    }

    #[test]
    fn native_and_modern_items_stay_retained_until_equip_adopts_them() {
        let item = |identifier: &str| WornItem {
            identifier: identifier.into(),
            metadata: 0,
            kind: crate::presentation::equipment::HeldKind::Sprite,
            dye_rgb: None,
        };
        for modern in [FILLED_MAP, "minecraft:crossbow", "minecraft:shield"] {
            let sword = item("minecraft:iron_sword");
            let modern = item(modern);
            for (old, new) in [(&sword, &modern), (&modern, &sword)] {
                let outgoing = retained_item(Some(&old.identifier), Some(new), Some(old)).unwrap();
                assert_eq!(outgoing.identifier, old.identifier);
                let incoming = retained_item(Some(&new.identifier), Some(new), Some(old)).unwrap();
                assert_eq!(incoming.identifier, new.identifier);
                assert!(retained_item(None, Some(new), Some(old)).is_none());
            }
        }
    }

    #[test]
    fn outgoing_main_use_is_idle_while_offhand_keeps_the_actual_owner_use() {
        let item = |identifier: &str| WornItem {
            identifier: identifier.into(),
            metadata: 0,
            kind: crate::presentation::equipment::HeldKind::Sprite,
            dye_rgb: None,
        };
        let owner = ActorEquipmentInput {
            main: Some(item("minecraft:apple")),
            off: Some(item("minecraft:shield")),
            ..Default::default()
        };
        let rendered = ActorEquipmentInput {
            main: Some(item(BOW)),
            ..owner.clone()
        };
        let timing = client_world::AttachableAnimationInput {
            first_person: true,
            frame_alpha: 0.75,
            animation_frame: 3,
            use_elapsed_ticks: Some(19),
            max_use_ticks: 32,
            hand_charged: true,
            ..Default::default()
        };
        let outgoing = super::super::hand::attachable_hand_input(&rendered, &owner, timing, false);
        assert_eq!(outgoing.animation_frame, 0);
        assert_eq!(outgoing.use_elapsed_ticks, None);
        assert!(!outgoing.hand_charged);
        assert!(outgoing.first_person);
        assert_eq!(outgoing.frame_alpha, timing.frame_alpha);
        assert_eq!(outgoing.owner_main_hand, Some(BOW));
        let off = super::super::hand::attachable_hand_input(&rendered, &owner, timing, true);
        assert_eq!(off.use_elapsed_ticks, timing.use_elapsed_ticks);
        assert_eq!(off.owner_main_hand, Some("minecraft:apple"));
        assert_eq!(off.owner_off_hand, Some("minecraft:shield"));
        assert_eq!(off.animation_frame, 0);
        assert!(!off.hand_charged);
        let adopted = super::super::hand::attachable_hand_input(&owner, &owner, timing, false);
        assert_eq!(adopted.animation_frame, timing.animation_frame);
        assert_eq!(adopted.use_elapsed_ticks, timing.use_elapsed_ticks);
        assert!(adopted.hand_charged);
    }

    #[test]
    fn mixed_native_map_swaps_publish_the_outgoing_mesh_until_adoption() {
        let ids = ["minecraft:iron_sword", FILLED_MAP];
        let sprites = [100, 200].map(|value| assets::IconSprite {
            width: 16,
            height: 16,
            rgba8: std::iter::repeat_n([value, 255, 255, 255], 16 * 16)
                .flatten()
                .collect::<Vec<_>>()
                .into(),
        });
        let mut entries = ids
            .iter()
            .enumerate()
            .map(|(index, id)| assets::IconEntry {
                identifier: (*id).into(),
                metadata: 0,
                sprite: index as u32,
            })
            .collect::<Vec<_>>();
        entries.sort_by(|left, right| left.identifier.cmp(&right.identifier));
        let icons = Arc::new(
            assets::RuntimeIconCatalog::decode(
                &assets::encode_icon_catalog([0; 32], &sprites, &entries).unwrap(),
            )
            .unwrap(),
        );
        let item = |identifier: &str| WornItem {
            identifier: identifier.into(),
            metadata: 0,
            kind: crate::presentation::equipment::HeldKind::Sprite,
            dye_rgb: None,
        };
        for (old, new) in [(ids[0], ids[1]), (ids[1], ids[0])] {
            let mut stream = head_stream();
            let mut feed = uploaded_skin_feed(false);
            feed.first_person = true;
            feed.main_hand = Some(old.into());
            stream.sync_local_player_pose(&feed);
            stream.advance_actor_interpolation_frame(6);
            let (mut equipment, artwork, _) = EquipmentRuntime::build(
                head_assets(),
                None,
                Arc::clone(&icons),
                None,
                None,
                render::ActorArtworkPages::default(),
            );
            let rig = stream.authority().actor_rig(1).unwrap();
            equipment.register_skin_rig(
                render_model::EntityRigId(rig.rig.0),
                rig.bone_names.to_vec(),
            );
            let mut cache = HandCache::default();
            cache.remember(&rig, Some(&item(old)));
            let actor = stream.authority().actor(1).unwrap();
            let presentation =
                crate::presentation::actors::entity_rig_presentation(&rig, actor, &artwork, 0.5)
                    .unwrap();
            let expected = equipment
                .first_person_java_item(
                    &presentation.submission,
                    &item(old),
                    JavaHand {
                        swing: 0.0,
                        equip: 1.0,
                        using: None,
                    },
                )
                .unwrap()
                .presentation
                .submission
                .input
                .rig;
            feed.main_hand = Some(new.into());
            stream.sync_local_player_pose(&feed);
            let selected = ActorEquipmentInput {
                main: Some(item(new)),
                ..Default::default()
            };
            for tick in 1..=3 {
                stream.advance_actor_interpolation_frame(1);
                let rig = stream.authority().actor_rig(1).unwrap();
                cache.remember(&rig, selected.main.as_ref());
                let actor = stream.authority().actor(1).unwrap();
                let presentation = crate::presentation::actors::entity_rig_presentation(
                    &rig, actor, &artwork, 0.5,
                )
                .unwrap();
                let source = hand_source(
                    HandInputs {
                        stream: &stream,
                        presentation,
                        equipment_input: &selected,
                        owner_equipment: &selected,
                        consume_ticks: None,
                        item_animation: None,
                        alpha: 0.5,
                        artwork: &artwork,
                        motion: Mat4::IDENTITY,
                    },
                    &mut equipment,
                    &mut cache,
                )
                .expect("the retained item must publish through a mixed swap");
                let drawn = source.items[0]
                    .as_ref()
                    .unwrap()
                    .0
                    .presentation
                    .submission
                    .input
                    .rig;
                if tick < 3 {
                    assert_eq!(drawn, expected, "{old} → {new} before adoption");
                } else {
                    assert_ne!(drawn, expected, "{old} → {new} after adoption");
                }
            }
        }
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
