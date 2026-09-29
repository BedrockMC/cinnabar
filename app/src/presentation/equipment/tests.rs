use std::sync::Arc;

use assets::IconSprite;
use bevy::math::{Quat, Vec3};
use render::{
    ACTOR_LAYER_BODY, ActorArtworkPages, ActorRenderIdentity, ActorRigRenderInput, ActorRigRoute,
    ActorRigSubmission, EntityRigId, EquipmentRaster, RenderBoneTransform,
};

use super::{
    armor::{bone_map, hidden_bone, pack_tint, remap_pose},
    atlas::{ATLAS_SIDE, SpriteAtlas},
    display::{ItemDisplay, attach_to_bone, held_sprite_display},
    runtime::{FirstPersonArms, layer_presentation},
};

fn sprite(side: u16, fill: u8) -> IconSprite {
    IconSprite {
        width: side,
        height: side,
        rgba8: vec![fill; usize::from(side) * usize::from(side) * 4].into(),
    }
}

fn bone(translation: [f32; 3], scale: f32) -> RenderBoneTransform {
    RenderBoneTransform {
        rotation: [0.0, 0.0, 0.0, 1.0],
        translation_scale: [translation[0], translation[1], translation[2], scale],
        axis_scale: render::UNIT_AXIS_SCALE,
    }
}

#[test]
fn atlas_places_sprites_without_overlap_and_copies_their_pixels() {
    let sprites = [sprite(16, 1), sprite(32, 2), sprite(16, 3)];
    let atlas = SpriteAtlas::pack(&sprites);
    assert_eq!(atlas.layers.len(), 1);
    let placements = atlas
        .placements
        .iter()
        .map(|placement| placement.expect("every sprite fits"))
        .collect::<Vec<_>>();
    for (index, placement) in placements.iter().enumerate() {
        let offset =
            (usize::from(placement.y) * usize::from(ATLAS_SIDE) + usize::from(placement.x)) * 4;
        assert_eq!(
            atlas.layers[placement.layer].rgba8[offset],
            sprites[index].rgba8[0]
        );
        for other in &placements[..index] {
            let disjoint = placement.x + placement.width <= other.x
                || other.x + other.width <= placement.x
                || placement.y + placement.height <= other.y
                || other.y + other.height <= placement.y;
            assert!(disjoint);
        }
    }
    let rect = placements[1].uv_rect();
    assert!(rect[0] >= 0.0 && rect[2] <= 1.0 && rect[2] > rect[0] && rect[3] > rect[1]);
}

#[test]
fn atlas_spills_into_extra_layers_and_skips_invalid_sprites() {
    let mut sprites = (0..600).map(|_| sprite(32, 9)).collect::<Vec<_>>();
    sprites.push(IconSprite {
        width: 4,
        height: 4,
        rgba8: Arc::from([0u8; 3]),
    });
    let atlas = SpriteAtlas::pack(&sprites);
    assert_eq!(atlas.layers.len(), 3);
    assert!(atlas.placements[..600].iter().all(Option::is_some));
    assert!(atlas.placements[600].is_none());
}

#[test]
fn attach_with_identity_display_passes_the_hand_pose_through() {
    let display = ItemDisplay {
        rotation: Quat::IDENTITY,
        translation: Vec3::ZERO,
        scale: 1.0,
    };
    let attached = attach_to_bone(bone([0.25, 0.5, -0.75], 1.0), display).unwrap();
    assert_eq!(attached.translation_scale, [0.25, 0.5, -0.75, 1.0]);
    assert_eq!(attached.rotation, [0.0, 0.0, 0.0, 1.0]);
}

#[test]
fn attach_scales_the_display_offset_by_the_hand_scale_and_hides_with_it() {
    let display = ItemDisplay {
        rotation: Quat::IDENTITY,
        translation: Vec3::new(0.0, 1.0, 0.0),
        scale: 0.5,
    };
    let attached = attach_to_bone(bone([0.0, 2.0, 0.0], 2.0), display).unwrap();
    assert_eq!(attached.translation_scale, [0.0, 4.0, 0.0, 1.0]);
    let hidden = attach_to_bone(bone([1.0, 1.0, 1.0], 0.0), display).unwrap();
    assert_eq!(hidden.translation_scale[3], 0.0);
    let mut bad = bone([0.0; 3], 1.0);
    bad.rotation = [0.0; 4];
    assert!(attach_to_bone(bad, display).is_none());
}

#[test]
fn hand_rotation_turns_the_display_offset() {
    let mut hand = bone([0.0; 3], 1.0);
    hand.rotation = Quat::from_rotation_z(std::f32::consts::FRAC_PI_2).to_array();
    let display = ItemDisplay {
        rotation: Quat::IDENTITY,
        translation: Vec3::X,
        scale: 1.0,
    };
    let attached = attach_to_bone(hand, display).unwrap();
    assert!((attached.translation_scale[1] - 1.0).abs() < 1e-5);
    assert!(attached.translation_scale[0].abs() < 1e-5);
}

#[test]
fn held_sprite_display_points_the_sprite_up_axis_up_and_width_forward() {
    let display = held_sprite_display();
    let up = display.rotation * Vec3::Y;
    let forward = display.rotation * Vec3::X;
    assert!((up - Vec3::Y).length() < 1e-5);
    assert!((forward - Vec3::NEG_Z).length() < 1e-5);
}

#[test]
fn armor_bones_follow_same_named_body_bones_and_hide_when_unmatched() {
    let names = |list: &[&str]| {
        list.iter()
            .map(|name| Box::<str>::from(*name))
            .collect::<Vec<_>>()
    };
    let body_names = names(&["root", "body", "head", "rightArm"]);
    let armor_names = names(&["body", "HEAD", "rightItem"]);
    let map = bone_map(&armor_names, &body_names);
    assert_eq!(map, vec![Some(1), Some(2), None]);
    let body = [
        bone([0.0; 3], 1.0),
        bone([1.0; 3], 1.0),
        bone([2.0; 3], 1.0),
        bone([3.0; 3], 1.0),
    ];
    let pose = remap_pose(&map, &body);
    assert_eq!(pose[0], body[1]);
    assert_eq!(pose[1], body[2]);
    assert_eq!(pose[2], hidden_bone());
}

#[test]
fn tint_packs_rgb_into_abgr_with_the_enabled_alpha() {
    assert_eq!(pack_tint(0x0011_2233), 0xff33_2211);
    assert_ne!(pack_tint(0), 0);
}

#[test]
fn equipment_layer_shares_the_body_identity_transform_and_generations() {
    let (_, locations) = ActorArtworkPages::default().with_equipment_rasters(&[EquipmentRaster {
        width: 2,
        height: 2,
        rgba8: vec![255; 16].into(),
    }]);
    let location = locations[0].unwrap();
    let body = ActorRigSubmission {
        input: ActorRigRenderInput {
            identity: ActorRenderIdentity {
                session_id: 1,
                dimension: 0,
                runtime_id: 2,
                spawn_revision: 3,
                ingress_sequence: 4,
                source_tick: None,
                movement_revision: 5,
                pose_generation: 6,
                layer: ACTOR_LAYER_BODY,
            },
            rig: EntityRigId(0),
            previous_bones: Arc::from([bone([0.0; 3], 1.0)]),
            current_bones: Arc::from([bone([0.0; 3], 1.0)]),
            completed_tick: 7,
            reset_generation: 8,
        },
        world_from_actor: [
            [1.0, 0.0, 0.0, 9.0],
            [0.0, 1.0, 0.0, 10.0],
            [0.0, 0.0, 1.0, 11.0],
        ],
        texture_layer: 0,
        route: ActorRigRoute::Compiled,
        tint: 0,
        overlay_rgba8: 0x6600_00ff,
    };
    let layer = layer_presentation(
        &body,
        4,
        EntityRigId(0x8000_0001),
        vec![bone([0.0; 3], 1.0)],
        vec![bone([1.0; 3], 1.0)],
        location,
        0xff00_00ff,
    );
    let submission = layer.submission;
    assert_eq!(submission.input.identity.layer, 4);
    assert_eq!(submission.input.identity.runtime_id, 2);
    assert_eq!(submission.input.completed_tick, 7);
    assert_eq!(submission.input.reset_generation, 8);
    assert_eq!(submission.world_from_actor, body.world_from_actor);
    assert_eq!(submission.texture_layer, location.layer());
    assert_eq!(submission.tint, 0xff00_00ff);
    assert_eq!(submission.overlay_rgba8, 0x6600_00ff);
}

#[test]
fn first_person_arms_follow_the_render_controller_visibility() {
    let arms = |main, off| FirstPersonArms::for_hands(main, off);
    assert_eq!(
        arms(None, None),
        FirstPersonArms {
            right: true,
            left: false
        }
    );
    assert_eq!(
        arms(Some("minecraft:diamond_sword"), None),
        FirstPersonArms {
            right: false,
            left: false
        }
    );
    assert_eq!(
        arms(Some("minecraft:filled_map"), None),
        FirstPersonArms {
            right: true,
            left: true
        }
    );
    assert!(!arms(Some("minecraft:filled_map"), Some("minecraft:shield")).left);
    assert!(arms(None, Some("minecraft:filled_map")).left);
}

#[test]
fn block_face_rects_tile_the_three_by_two_sheet() {
    let rects = super::blocks::face_rects([0.0, 0.0, 0.75, 0.5]);
    assert_eq!(rects[0], [0.0, 0.0, 0.25, 0.25]);
    assert_eq!(rects[5], [0.5, 0.25, 0.75, 0.5]);
}

#[test]
fn elytra_wings_hang_off_the_body_at_their_literal_offsets() {
    use assets::{AttachablePose, AttachablePoseBone, ItemDisplayScalar};
    let scalar = |value: f32| ItemDisplayScalar::new(value).unwrap();
    let uniform = |value: f32| Some([scalar(value); 3]);
    let pose = AttachablePose {
        key: "default".into(),
        bones: Box::new([
            AttachablePoseBone {
                bone: "body".into(),
                translation: None,
                rotation: None,
                scale: uniform(1.5),
            },
            AttachablePoseBone {
                bone: "left_wing".into(),
                translation: Some([scalar(4.0), scalar(8.0), scalar(-16.0)]),
                rotation: None,
                scale: Some([scalar(1.0), scalar(1.0), scalar(2.0)]),
            },
        ]),
    };
    let names = ["body", "left_wing", "right_wing"]
        .map(Box::<str>::from)
        .to_vec();
    let posed = super::elytra::pose(&names, &pose, bone([0.0, 1.0, 0.0], 1.0));
    assert_eq!(posed[0].translation_scale, [0.0, 1.0, 0.0, 1.5]);
    // X is mirrored, offsets scale with the body, and the wing stacks its own axis scale.
    let wing = posed[1].translation_scale;
    assert!((wing[0] + 4.0 / 16.0 * 1.5).abs() < 1e-6);
    assert!((wing[1] - (1.0 + 8.0 / 16.0 * 1.5)).abs() < 1e-6);
    assert!((wing[2] + 1.0 * 1.5).abs() < 1e-6);
    assert_eq!(wing[3], 1.5);
    assert_eq!(posed[1].axis_scale, [1.0, 1.0, 2.0, 1.0]);
    assert_eq!(posed[2], hidden_bone());
}

#[test]
fn head_items_map_to_their_skull_kinds_and_others_to_none() {
    use render::SkullKind;
    let kind = super::runtime::skull_kind;
    assert_eq!(kind("minecraft:zombie_head"), Some(SkullKind::Zombie));
    assert_eq!(kind("minecraft:skeleton_skull"), Some(SkullKind::Skeleton));
    assert_eq!(kind("minecraft:player_head"), Some(SkullKind::Player));
    assert_eq!(kind("minecraft:dragon_head"), None);
    assert_eq!(kind("minecraft:carved_pumpkin"), None);
}

#[test]
fn attachable_bone_sits_at_its_pivot_plus_the_mirrored_literal_offset() {
    use super::attachable::{BoneChannels, attach};
    let hand = bone([1.0, 1.0, 1.0], 2.0);
    let channels = BoneChannels {
        translation: [16.0, 8.0, -16.0],
        rotation: [0.0; 3],
        scale: [1.0, -1.0, -1.0],
    };
    let posed = attach(hand, [0.0, 1.5, 0.0], channels).unwrap();
    // Pivot and offset are in the hand frame, so the hand scale (2) stretches them.
    assert_eq!(
        posed.translation_scale,
        [1.0 - 2.0, 1.0 + 2.0 * (1.5 + 0.5), 1.0 - 2.0, 2.0]
    );
    assert_eq!(posed.axis_scale, [1.0, -1.0, -1.0, 1.0]);
    let mut broken = hand;
    broken.rotation = [0.0; 4];
    assert!(attach(broken, [0.0; 3], channels).is_none());
}

fn local_carrier(name: &str) -> Option<Vec<u8>> {
    std::fs::read(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../.local/assets/compiled")
            .join(name),
    )
    .ok()
}

// Local-only: which held items and armor pieces the real carriers can draw on a player body.
#[test]
fn real_carriers_draw_armor_and_report_each_held_item() {
    use super::runtime::{ActorEquipmentInput, EquipmentRuntime, HeldKind, WornItem};
    let (Some(entities), Some(icons), Some(equipment)) = (
        local_carrier("vanilla-v1.mcbeent"),
        local_carrier("vanilla-v1.mcbeico"),
        local_carrier("vanilla-v1.mcbeeqp"),
    ) else {
        return;
    };
    let entities = Arc::new(assets::RuntimeEntityAssets::decode(&entities).unwrap());
    let icons = Arc::new(assets::RuntimeIconCatalog::decode(&icons).unwrap());
    let catalog = Arc::new(assets::RuntimeEquipmentCatalog::decode(&equipment).unwrap());
    let (mut runtime, _, _) = EquipmentRuntime::build(
        entities,
        Some(catalog),
        icons,
        None,
        None,
        ActorArtworkPages::default(),
    );
    let names = [
        "root",
        "body",
        "waist",
        "head",
        "hat",
        "rightArm",
        "leftArm",
        "rightLeg",
        "leftLeg",
        "rightItem",
        "leftItem",
    ]
    .map(Box::<str>::from)
    .to_vec();
    let rig = EntityRigId(0x7000_0000);
    runtime.register_skin_rig(rig, names.clone());
    let pose: Arc<[RenderBoneTransform]> = names.iter().map(|_| bone([0.0; 3], 1.0)).collect();
    let body = ActorRigSubmission {
        input: ActorRigRenderInput {
            identity: ActorRenderIdentity {
                session_id: 1,
                dimension: 0,
                runtime_id: 2,
                spawn_revision: 1,
                ingress_sequence: 1,
                source_tick: None,
                movement_revision: 0,
                pose_generation: 0,
                layer: ACTOR_LAYER_BODY,
            },
            rig,
            previous_bones: Arc::clone(&pose),
            current_bones: pose,
            completed_tick: 0,
            reset_generation: 0,
        },
        world_from_actor: [
            [1.0, 0.0, 0.0, 0.0],
            [0.0, 1.0, 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
        ],
        texture_layer: 0,
        route: ActorRigRoute::Compiled,
        tint: 0,
        overlay_rgba8: 0,
    };
    let worn = |identifier: &str| WornItem {
        identifier: Arc::from(identifier),
        metadata: 0,
        kind: HeldKind::Sprite,
        dye_rgb: None,
    };
    let armor = ActorEquipmentInput {
        armor: [
            "minecraft:diamond_helmet",
            "minecraft:diamond_chestplate",
            "minecraft:diamond_leggings",
            "minecraft:diamond_boots",
        ]
        .map(|identifier| Some(worn(identifier))),
        ..ActorEquipmentInput::default()
    };
    assert_eq!(runtime.layers_for(&body, &armor).len(), 4);
    let held = [
        "minecraft:ender_pearl",
        "minecraft:diamond_sword",
        "minecraft:golden_apple",
    ]
    .map(|identifier| {
        let input = ActorEquipmentInput {
            main: Some(worn(identifier)),
            ..ActorEquipmentInput::default()
        };
        (identifier, runtime.layers_for(&body, &input).len())
    });
    eprintln!("{held:?}");
    assert_eq!(held[0].1, 1);
}
