//! Worn wings keep authored poses and textures while riding the player's body.

use super::{pack_runtime, player_body};
use crate::presentation::equipment::{
    display::LAYER_CHESTPLATE,
    runtime::{ActorEquipmentInput, EquipmentAnimation, EquipmentRuntime, HeldKind, WornItem},
};
use assets::{EntityRenderMaterial, EntityRigFallback};
use client_world::{
    ActorAnimationVariables, ActorLifetimeId, ActorRigSnapshot, ActorSnapshot, HandPhase,
    ItemAnimationState, WorldAuthority,
};
use protocol::{
    ActorEvent, ActorKind, ActorMetadataValue, ActorSpawnEvent, WorldBootstrap, WorldEvent,
};
use render::{ActorArtworkPages, ActorRigSubmission};
use std::sync::Arc;

/// A solid synthetic raster in the wing geometry's native UV dimensions.
fn wing_texture() -> Vec<u8> {
    let image = image::RgbaImage::from_pixel(64, 32, image::Rgba([200, 40, 40, 255]));
    let mut bytes = std::io::Cursor::new(Vec::new());
    image.write_to(&mut bytes, image::ImageFormat::Png).unwrap();
    bytes.into_inner()
}

/// A small authored wing rig with independent folded, crouching and velocity-driven flight poses.
fn wing_pack() -> Vec<(Box<str>, Vec<u8>)> {
    let geometry = assets::ELYTRA_GEOMETRY_IDENTIFIER;
    vec![
        ("attachables/elytra.json".into(), serde_json::to_vec(&serde_json::json!({
            "format_version":"1.10.0","minecraft:attachable":{"description":{
                "identifier":"minecraft:elytra","materials":{"default":"elytra","enchanted":"elytra_glint"},
                "textures":{"default":"textures/models/wings"},"geometry":{"default":geometry},
                "animations":{"controller":"controller.animation.wings","folded":"animation.wings.folded",
                    "crouching":"animation.wings.crouching","flight":"animation.wings.flight"},
                "scripts":{"animate":["controller"]},"render_controllers":["controller.render.wings"]
            }}
        })).unwrap()),
        ("models/entity/wings.json".into(), serde_json::to_vec(&serde_json::json!({
            "format_version":"1.12.0","minecraft:geometry":[{
                "description":{"identifier":geometry,"texture_width":64,"texture_height":32},
                "bones":[{"name":"body","pivot":[0,24,0]},
                    {"name":"left_wing","parent":"body","pivot":[0,24,0],
                        "cubes":[{"origin":[-10,0,0],"size":[10,20,2],"uv":[22,0]}]},
                    {"name":"right_wing","parent":"body","pivot":[0,24,0],"mirror":true,
                        "cubes":[{"origin":[0,0,0],"size":[10,20,2],"uv":[22,0]}]}]
            }]
        })).unwrap()),
        ("animations/wings.json".into(), br#"{"format_version":"1.8.0","animations":{
            "animation.wings.folded":{"loop":true,"bones":{
                "left_wing":{"position":[4,0,-2],"rotation":[15,0,-13]},
                "right_wing":{"position":[-4,0,-2],"rotation":[15,0,13]}}},
            "animation.wings.crouching":{"loop":true,"bones":{
                "left_wing":{"position":[4,-3,-2],"rotation":[40,5,-40]},
                "right_wing":{"position":[-4,-3,-2],"rotation":[40,-5,40]}}},
            "animation.wings.flight":{"loop":true,"bones":{
                "left_wing":{"rotation":["15 + query.vertical_speed * 20",0,-75]},
                "right_wing":{"rotation":["15 + query.vertical_speed * 20",0,75]}}}
        }}"#.to_vec()),
        ("animation_controllers/wings.json".into(), br#"{"format_version":"1.10.0","animation_controllers":{
            "controller.animation.wings":{"initial_state":"folded","states":{
                "folded":{"animations":["folded"],"transitions":[{"flight":"query.is_gliding"},{"crouching":"query.is_sneaking"}]},
                "flight":{"animations":["flight"],"transitions":[{"folded":"!query.is_gliding"}]},
                "crouching":{"animations":["crouching"],"transitions":[{"flight":"query.is_gliding"},{"folded":"!query.is_sneaking"}]}
            }}
        }}"#.to_vec()),
        ("render_controllers/wings.json".into(), br#"{"format_version":"1.8.0","render_controllers":{
            "controller.render.wings":{"geometry":"Geometry.default","textures":["Texture.default"],
                "materials":[{"*":"variable.is_enchanted ? Material.enchanted : Material.default"}]}
        }}"#.to_vec()),
        ("textures/models/wings.png".into(), wing_texture()),
    ]
}

/// Produces a real player snapshot through the public event admission path.
fn owner() -> ActorSnapshot {
    let mut world = WorldAuthority::new(
        WorldBootstrap {
            local_player_unique_id: 1,
            local_player_runtime_id: 1,
            dimension: 0,
            player_position: [0.0; 3],
            world_spawn_position: [0; 3],
            air_network_id: 0,
            block_network_ids_are_hashes: false,
        },
        Arc::new(assets::RuntimeAssets::diagnostic()),
        None,
        [0.0; 3],
        None,
    );
    world
        .apply_ordered_event(
            WorldEvent::Actor(ActorEvent::Spawn(ActorSpawnEvent {
                dimension: 0,
                unique_id: 2,
                runtime_id: 2,
                kind: ActorKind::Player {
                    uuid: [2; 16],
                    username: "wing-test".into(),
                },
                position: [0.0; 3],
                velocity: [0.0; 3],
                pitch: 0.0,
                yaw: 0.0,
                head_yaw: 0.0,
                body_yaw: 0.0,
                held_item: Default::default(),
                metadata: Arc::from([]),
                attributes: Arc::from([]),
                properties: Arc::from([]),
                links: Arc::from([]),
            })),
            Some(1),
        )
        .unwrap();
    world.actor(2).unwrap().clone()
}

/// Supplies tick-owned actor identity and body-name bindings to the authored attachable.
fn owner_rig<'a>(owner: &ActorSnapshot, names: &'a [Box<str>], tick: u64) -> ActorRigSnapshot<'a> {
    ActorRigSnapshot {
        actor: ActorLifetimeId {
            session_id: 1,
            dimension: 0,
            runtime_id: owner.runtime_id,
            spawn_revision: owner.spawn_revision,
        },
        rig: client_world::EntityRigId(0),
        previous: &[],
        current: &[],
        rest: &[],
        rest_completed_tick: tick,
        rest_reset_generation: 0,
        completed_tick: tick,
        reset_generation: 0,
        fallback: EntityRigFallback::Skip,
        scale: 1.0,
        axis_scale: [1.0; 3],
        previous_body_yaw: 0.0,
        body_yaw: 0.0,
        render: &[],
        bone_names: names,
        skin_geometry: None,
        skin_layers: &[],
        hand: [HandPhase::default(); 2],
        item_animation: [ItemAnimationState::default(); 2],
        off_hand_animation: [ItemAnimationState::default(); 2],
        animation_variables: ActorAnimationVariables::default(),
    }
}

/// Equips only the chest slot so any published layer must belong to the wings.
fn worn(enchanted: bool) -> ActorEquipmentInput {
    ActorEquipmentInput {
        armor: [
            None,
            Some(WornItem {
                identifier: Arc::from("minecraft:elytra"),
                metadata: 0,
                kind: HeldKind::Other,
                dye_rgb: None,
                enchanted,
            }),
            None,
            None,
        ],
        ..Default::default()
    }
}

/// Evaluates one full-alpha equipment frame for a controller's current owner state.
fn layers(
    runtime: &mut EquipmentRuntime,
    body: &ActorRigSubmission,
    owner: &ActorSnapshot,
    input: &ActorEquipmentInput,
    tick: u64,
) -> Vec<crate::presentation::equipment::runtime::EquipmentPresentation> {
    let names = [Box::<str>::from("body")];
    let rig = owner_rig(owner, &names, tick);
    runtime.layers_for(
        body,
        input,
        Some(EquipmentAnimation {
            owner,
            rig: &rig,
            frame_alpha: 1.0,
        }),
    )
}

/// Resolves the selected artwork texel without depending on atlas page layout.
fn selected_pixel(
    pages: &ActorArtworkPages,
    layer: &crate::presentation::equipment::runtime::EquipmentPresentation,
) -> [u8; 4] {
    let page = &pages.pages()[usize::from(layer.location.page().checked_sub(1).unwrap())];
    let (width, height) = page.dimensions();
    let start = layer.location.layer() as usize * usize::from(width) * usize::from(height) * 4;
    page.pixels()[start..start + 4].try_into().unwrap()
}

#[test]
fn chest_elytra_draws_native_wings_and_follows_folded_crouching_and_flight_controller() {
    let (mut runtime, pages) = pack_runtime(wing_pack());
    let body = player_body(&mut runtime);
    let mut owner = owner();
    let input = worn(false);
    let folded = layers(&mut runtime, &body, &owner, &input, 1);
    assert_eq!(
        folded.len(),
        1,
        "chest-slot elytra must publish a worn draw"
    );
    assert_eq!(folded[0].submission.input.identity.layer, LAYER_CHESTPLATE);
    assert_ne!(folded[0].submission.input.rig, body.input.rig);
    assert_eq!(folded[0].submission.input.current_bones.len(), 3);
    assert_eq!(selected_pixel(&pages, &folded[0]), [200, 40, 40, 255]);
    assert!(folded[0].submission.material.state.unwrap().alpha_test);
    assert!(!folded[0].submission.material.state.unwrap().cull);

    owner.metadata.insert(0, ActorMetadataValue::Flags(1 << 1));
    let crouching = layers(&mut runtime, &body, &owner, &input, 2);
    assert_eq!(crouching.len(), 1);
    assert_ne!(
        folded[0].submission.input.current_bones,
        crouching[0].submission.input.current_bones
    );

    owner.metadata.insert(0, ActorMetadataValue::Flags(1 << 32));
    let flight = layers(&mut runtime, &body, &owner, &input, 3);
    assert_eq!(flight.len(), 1);
    assert_ne!(
        folded[0].submission.input.current_bones,
        flight[0].submission.input.current_bones
    );
    assert_ne!(
        crouching[0].submission.input.current_bones,
        flight[0].submission.input.current_bones
    );
    owner.metadata.insert(0, ActorMetadataValue::Flags(0));
    let landed = layers(&mut runtime, &body, &owner, &input, 5);
    assert_eq!(
        folded[0].submission.input.current_bones,
        landed[0].submission.input.current_bones
    );
}

#[test]
fn enchanted_wings_keep_the_chest_texture_and_reuse_unchanged_pose_without_new_geometry() {
    let (mut runtime, pages) = pack_runtime(wing_pack());
    let body = player_body(&mut runtime);
    let owner = owner();
    let plain = layers(&mut runtime, &body, &owner, &worn(false), 1);
    let enchanted = layers(&mut runtime, &body, &owner, &worn(true), 2);
    assert_eq!(enchanted.len(), 1);
    assert_eq!(
        enchanted[0].submission.material.kind,
        EntityRenderMaterial::Glint
    );
    assert_eq!(enchanted[0].location, plain[0].location);
    assert_eq!(selected_pixel(&pages, &enchanted[0]), [200, 40, 40, 255]);
    runtime.take_pending_geometries();
    let unchanged = layers(&mut runtime, &body, &owner, &worn(true), 2);
    assert!(Arc::ptr_eq(
        &enchanted[0].submission.input.current_bones,
        &unchanged[0].submission.input.current_bones
    ));
    assert_eq!(
        enchanted[0].submission.input.rig,
        unchanged[0].submission.input.rig
    );
    assert!(runtime.take_pending_geometries().is_empty());
}

#[test]
fn worn_transition_blends_short_wing_angles_and_preserves_identical_scale() {
    let mut files = wing_pack();
    for (path, bytes) in &mut files {
        if path.as_ref() == "animation_controllers/wings.json" {
            let mut document: serde_json::Value = serde_json::from_slice(bytes).unwrap();
            for state in document["animation_controllers"]["controller.animation.wings"]["states"]
                .as_object_mut()
                .unwrap()
                .values_mut()
            {
                state["blend_transition"] = serde_json::json!(0.1);
                state["blend_via_shortest_path"] = serde_json::json!(true);
            }
            *bytes = serde_json::to_vec(&document).unwrap();
        }
        if path.as_ref() == "animations/wings.json" {
            let mut document: serde_json::Value = serde_json::from_slice(bytes).unwrap();
            for animation in document["animations"].as_object_mut().unwrap().values_mut() {
                animation["bones"]["body"] = serde_json::json!({"scale": 1.067});
            }
            document["animations"]["animation.wings.folded"]["bones"]["left_wing"]["rotation"] =
                serde_json::json!([15, 0, 170]);
            document["animations"]["animation.wings.flight"]["bones"]["left_wing"]["rotation"] =
                serde_json::json!([15, 0, -170]);
            *bytes = serde_json::to_vec(&document).unwrap();
        }
    }
    let (mut runtime, _) = pack_runtime(files);
    let body = player_body(&mut runtime);
    let mut owner = owner();
    let input = worn(false);
    let folded = layers(&mut runtime, &body, &owner, &input, 1);
    owner.metadata.insert(0, ActorMetadataValue::Flags(1 << 32));
    let halfway = layers(&mut runtime, &body, &owner, &input, 2);
    let spread = layers(&mut runtime, &body, &owner, &input, 4);
    assert_ne!(
        halfway[0].submission.input.current_bones,
        folded[0].submission.input.current_bones
    );
    assert_ne!(
        halfway[0].submission.input.current_bones,
        spread[0].submission.input.current_bones
    );
    let wing = halfway[0].submission.input.current_bones[1];
    let expected = render_model::equipment::authored_rotation([15.0, 0.0, 180.0]);
    assert!(bevy::math::Quat::from_array(wing.rotation).abs_diff_eq(expected, 1e-5));
    assert!(
        (wing.axis_scale[0] - 1.067).abs() < 1e-5,
        "matching state scales must stay unchanged while blending"
    );
}

#[test]
fn worn_wings_follow_the_supplied_body_pose_without_changing_the_pack_pose() {
    let (mut runtime, _) = pack_runtime(wing_pack());
    let body = player_body(&mut runtime);
    let owner = owner();
    let input = worn(false);
    let rest = layers(&mut runtime, &body, &owner, &input, 1);
    let body_index = 1;
    let turn = bevy::math::Quat::from_rotation_x(0.5);
    let mut moved = body.clone();
    let mut pose = body.input.current_bones.to_vec();
    pose[body_index].rotation = turn.to_array();
    moved.input.previous_bones = Arc::from(pose.clone());
    moved.input.current_bones = Arc::from(pose);
    let attached = layers(&mut runtime, &moved, &owner, &input, 2);
    assert_eq!(attached[0].location, rest[0].location);
    assert_eq!(
        attached[0].submission.input.rig,
        rest[0].submission.input.rig
    );
    for (posed, original) in attached[0]
        .submission
        .input
        .current_bones
        .iter()
        .zip(rest[0].submission.input.current_bones.iter())
    {
        let expected = turn * bevy::math::Quat::from_array(original.rotation);
        assert!(bevy::math::Quat::from_array(posed.rotation).abs_diff_eq(expected, 1e-5));
    }
}

#[test]
fn worn_controller_takes_one_transition_per_frame_when_landing_while_sneaking() {
    let (mut runtime, _) = pack_runtime(wing_pack());
    let body = player_body(&mut runtime);
    let mut owner = owner();
    let input = worn(false);
    let folded = layers(&mut runtime, &body, &owner, &input, 1);
    owner.metadata.insert(0, ActorMetadataValue::Flags(1 << 32));
    layers(&mut runtime, &body, &owner, &input, 2);
    owner.metadata.insert(0, ActorMetadataValue::Flags(1 << 1));
    let landing = layers(&mut runtime, &body, &owner, &input, 3);
    assert_eq!(
        landing[0].submission.input.current_bones,
        folded[0].submission.input.current_bones
    );
    let crouching = layers(&mut runtime, &body, &owner, &input, 4);
    assert_ne!(
        crouching[0].submission.input.current_bones,
        folded[0].submission.input.current_bones
    );
}

#[test]
fn installed_elytra_carrier_publishes_worn_wings_with_the_pack_controller() {
    let (Some(entities), Some(icons), Some(equipment)) = (
        super::local_carrier("vanilla-v1.mcbeent"),
        super::local_carrier("vanilla-v1.mcbeico"),
        super::local_carrier("vanilla-v1.mcbeeqp"),
    ) else {
        return;
    };
    let entities = Arc::new(assets::RuntimeEntityAssets::decode(&entities).unwrap());
    let icons = Arc::new(assets::RuntimeIconCatalog::decode(&icons).unwrap());
    let catalog = Arc::new(assets::RuntimeEquipmentCatalog::decode(&equipment).unwrap());
    assert!(
        entities
            .attachable_rig_binding("minecraft:elytra")
            .is_some()
    );
    let (mut runtime, _, _) = EquipmentRuntime::build(
        entities,
        Some(catalog),
        icons,
        None,
        None,
        ActorArtworkPages::default(),
    );
    let body = player_body(&mut runtime);
    let owner = owner();
    let draws = layers(&mut runtime, &body, &owner, &worn(false), 1);
    assert_eq!(
        draws.len(),
        1,
        "the installed armor render controller must keep the base wings texture"
    );
    assert_eq!(draws[0].submission.input.current_bones.len(), 3);
    let enchanted = layers(&mut runtime, &body, &owner, &worn(true), 2);
    assert_eq!(enchanted.len(), 1);
    assert_eq!(
        enchanted[0].submission.material.kind,
        EntityRenderMaterial::Glint
    );
    assert_eq!(enchanted[0].location, draws[0].location);
}
