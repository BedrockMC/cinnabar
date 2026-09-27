//! Independently authored player-shaped pack compiled end to end: rig scripts, a nested
//! root controller, expression channels, and actor-state queries drive the pose.
use assets::{EntityRigFallback, RuntimeAssets, RuntimeEntityAssets, encode_entity_blob};
use client_world::{BoneTransform, WorldStream};
use protocol::{
    ActorActionEvent, ActorActionKind, ActorEvent, ActorKind, ActorMetadata,
    ActorMetadataUpdateEvent, ActorMetadataValue, ActorSpawnEvent, ItemActorEvent, MovePlayerEvent,
    MovePlayerMode, WorldBootstrap, WorldEvent,
};
use std::{fs, path::PathBuf, sync::Arc};

const ENTITY: &str = r#"{"format_version":"1.26.0","minecraft:client_entity":{"description":{
 "identifier":"minecraft:player",
 "materials":{"default":"entity_alphatest"},
 "textures":{"default":"textures/entity/steve"},
 "geometry":{"default":"geometry.humanoid.custom"},
 "scripts":{"scale":"0.9375",
  "initialize":["variable.is_holding_right = 0.0;"],
  "pre_animation":["variable.tcos0 = (Math.cos(query.modified_distance_moved * 38.17) * query.modified_move_speed / variable.gliding_speed_value) * 57.3;"],
  "animate":["root"]},
 "animations":{"root":"controller.animation.player.root","look":"controller.animation.humanoid.look_at_target","look_default":"animation.humanoid.look_at_target.default","legs":"animation.player.move.legs","attack":"animation.player.attack.rotations","sneak":"animation.player.sneaking","unused":"controller.animation.player.base"},
 "render_controllers":[{"controller.render.player.first_person":"variable.is_first_person"},{"controller.render.player.third_person":"!variable.is_first_person"}]}}}"#;

const GEOMETRY: &str = r#"{"format_version":"1.12.0","minecraft:geometry":[{"description":{"identifier":"geometry.humanoid.custom","texture_width":64,"texture_height":64},"bones":[
 {"name":"root","pivot":[0,0,0]},
 {"name":"body","parent":"root","pivot":[0,24,0],"cubes":[{"origin":[-4,12,-2],"size":[8,12,4],"uv":[16,16]}]},
 {"name":"head","parent":"body","pivot":[0,24,0],"cubes":[{"origin":[-4,24,-4],"size":[8,8,8],"uv":[0,0]}]},
 {"name":"rightArm","parent":"body","pivot":[-5,22,0],"cubes":[{"origin":[-8,12,-2],"size":[4,12,4],"uv":[40,16]}]},
 {"name":"leftArm","parent":"body","pivot":[5,22,0],"cubes":[{"origin":[4,12,-2],"size":[4,12,4],"uv":[32,48]}]},
 {"name":"rightLeg","parent":"root","pivot":[-1.9,12,0],"cubes":[{"origin":[-3.9,0,-2],"size":[4,12,4],"uv":[0,16]}]},
 {"name":"leftLeg","parent":"root","pivot":[1.9,12,0],"cubes":[{"origin":[-0.1,0,-2],"size":[4,12,4],"uv":[16,48]}]}]}]}"#;

const ANIMATIONS: &str = r#"{"format_version":"1.8.0","animations":{
 "animation.humanoid.look_at_target.default":{"loop":true,"bones":{"head":{"relative_to":{"rotation":"entity"},"rotation":["query.target_x_rotation","query.target_y_rotation",0.0]}}},
 "animation.player.move.legs":{"loop":true,"bones":{"leftleg":{"rotation":["variable.tcos0 * -1.4",0.0,0.0]},"rightleg":{"rotation":["variable.tcos0 * 1.4",0.0,0.0]}}},
 "animation.player.attack.rotations":{"loop":true,"bones":{"rightarm":{"rotation":["-math.sin(variable.attack_time * 180) * 30",0.0,0.0]}}},
 "animation.player.sneaking":{"loop":true,"bones":{"root":{"rotation":["28.0 - this",0.0,0.0]}}}}}"#;

const CONTROLLERS: &str = r#"{"format_version":"1.10.0","animation_controllers":{
 "controller.animation.player.root":{"initial_state":"first_person","states":{
  "first_person":{"transitions":[{"third_person":"!variable.is_first_person"}]},
  "third_person":{"animations":[{"look":"!query.is_sleeping && !query.is_emoting"},"legs",{"attack":"variable.attack_time > 0.0"},{"sneak":"query.is_sneaking"},{"missing_clip":"query.get_equipped_item_name == 'bow'"}],
   "transitions":[{"first_person":"variable.is_first_person"}]}}},
 "controller.animation.humanoid.look_at_target":{"initial_state":"default","states":{"default":{"animations":["look_default"]}}}}}"#;

const RENDER: &str = r#"{"format_version":"1.8.0","render_controllers":{
 "controller.render.player.first_person":{"geometry":"Geometry.default","materials":[{"*":"Material.default"}],"textures":["Texture.default"]},
 "controller.render.player.third_person":{"geometry":"Geometry.default","materials":[{"*":"Material.default"}],"textures":["Texture.default"]}}}"#;

struct Pack(PathBuf);

impl Pack {
    fn new() -> Self {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let root = std::env::temp_dir().join(format!(
            "player-animation-test-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        let _ = fs::remove_dir_all(&root);
        for (path, contents) in [
            ("entity/player.entity.json", ENTITY),
            ("models/entity/player.geo.json", GEOMETRY),
            ("animations/player.animation.json", ANIMATIONS),
            (
                "animation_controllers/player.animation_controllers.json",
                CONTROLLERS,
            ),
            ("render_controllers/player.render_controllers.json", RENDER),
        ] {
            let path = root.join(path);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, contents).unwrap();
        }
        fs::create_dir_all(root.join("textures/entity")).unwrap();
        image::RgbaImage::from_pixel(64, 64, image::Rgba([90, 60, 40, 255]))
            .save(root.join("textures/entity/steve.png"))
            .unwrap();
        Self(root)
    }
}

impl Drop for Pack {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn entities() -> Arc<RuntimeEntityAssets> {
    let pack = Pack::new();
    let manifest = include_bytes!("../../../assets/vanilla-source.json");
    let compiled = asset_compiler::compile_entity_assets(&pack.0, manifest).unwrap();
    Arc::new(RuntimeEntityAssets::decode(&encode_entity_blob(&compiled).unwrap()).unwrap())
}

fn stream(entities: Arc<RuntimeEntityAssets>) -> WorldStream {
    WorldStream::new_with_asset_sets(
        WorldBootstrap {
            local_player_unique_id: 1,
            dimension: 0,
            local_player_runtime_id: 1,
            player_position: [0.0, 64.0, 0.0],
            world_spawn_position: [0, 64, 0],
            air_network_id: 0,
            block_network_ids_are_hashes: false,
        },
        Arc::new(RuntimeAssets::diagnostic()),
        entities,
        [0.0, 64.0, 0.0],
        None,
    )
}

fn spawn_player() -> WorldEvent {
    WorldEvent::Actor(ActorEvent::Spawn(ActorSpawnEvent {
        dimension: 0,
        unique_id: -42,
        runtime_id: 42,
        kind: ActorKind::Player {
            uuid: [7; 16],
            username: "remote".into(),
        },
        position: [0.0, 64.0, 0.0],
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
    }))
}

fn move_player(x: f32, yaw: f32, pitch: f32, tick: u64) -> WorldEvent {
    WorldEvent::MovePlayer(MovePlayerEvent {
        runtime_id: 42,
        position: [x, 64.0 + protocol::PLAYER_NETWORK_OFFSET, 0.0],
        pitch,
        yaw,
        head_yaw: yaw,
        mode: MovePlayerMode::Normal,
        on_ground: true,
        teleported: false,
        source_tick: tick,
    })
}

fn bone(world: &WorldStream, entities: &RuntimeEntityAssets, name: &str) -> BoneTransform {
    let rig = world.actor_rig(42).unwrap();
    let geometry = entities.rig_geometries()[rig.rig.0 as usize].geometry as usize;
    let index = entities.geometries()[geometry]
        .bones
        .iter()
        .position(|bone| bone.name.eq_ignore_ascii_case(name))
        .unwrap();
    rig.current[index]
}

fn turned(transform: BoneTransform) -> bool {
    transform.rotation[3] < 0.9999
}

#[test]
fn vanilla_shaped_player_rig_resolves_animated_with_its_authored_scale() {
    let entities = entities();
    let mut world = stream(Arc::clone(&entities));
    world.submit(1, spawn_player()).unwrap();
    let rig = world.actor_rig(42).unwrap();
    assert_eq!(rig.fallback, EntityRigFallback::Skip);
    assert_eq!(rig.scale, 0.9375);
    world.advance_actor_interpolation_ticks(2);
    for name in ["leftLeg", "rightLeg", "rightArm", "head", "root"] {
        assert!(!turned(bone(&world, &entities, name)), "{name} rests");
    }
}

#[test]
fn walking_swings_the_legs_in_opposition_and_stopping_settles_them() {
    let entities = entities();
    let mut world = stream(Arc::clone(&entities));
    world.submit(1, spawn_player()).unwrap();
    for step in 1..=6_u64 {
        world
            .submit(step + 1, move_player(step as f32 * 0.25, -90.0, 0.0, step))
            .unwrap();
        world.advance_actor_interpolation_ticks(1);
    }
    let left = bone(&world, &entities, "leftLeg");
    let right = bone(&world, &entities, "rightLeg");
    assert!(turned(left) && turned(right));
    assert!(left.rotation[0] * right.rotation[0] < 0.0, "legs oppose");
    world.advance_actor_interpolation_ticks(60);
    assert!(!turned(bone(&world, &entities, "leftLeg")));
}

#[test]
fn head_pitch_turns_the_head_through_the_nested_look_controller() {
    let entities = entities();
    let mut world = stream(Arc::clone(&entities));
    world.submit(1, spawn_player()).unwrap();
    world.submit(2, move_player(0.0, 0.0, 30.0, 1)).unwrap();
    world.advance_actor_interpolation_ticks(4);
    assert!(turned(bone(&world, &entities, "head")));
    assert!(!turned(bone(&world, &entities, "leftLeg")));
}

#[test]
fn arm_swing_action_animates_the_attack_then_returns_to_rest() {
    let entities = entities();
    let mut world = stream(Arc::clone(&entities));
    world.submit(1, spawn_player()).unwrap();
    world.advance_actor_interpolation_ticks(1);
    world
        .submit(
            2,
            WorldEvent::ItemActor(ItemActorEvent::Action(ActorActionEvent {
                actor_runtime_ids: Arc::from([42_u64]),
                kind: ActorActionKind::SwingArm,
                data: 0.0,
                swing_source: None,
            })),
        )
        .unwrap();
    world.advance_actor_interpolation_ticks(3);
    assert!(turned(bone(&world, &entities, "rightArm")));
    world.advance_actor_interpolation_ticks(6);
    assert!(!turned(bone(&world, &entities, "rightArm")));
}

#[test]
fn sneaking_flag_overrides_the_root_tilt_through_this() {
    let entities = entities();
    let mut world = stream(Arc::clone(&entities));
    world.submit(1, spawn_player()).unwrap();
    world
        .submit(
            2,
            WorldEvent::Actor(ActorEvent::Metadata(ActorMetadataUpdateEvent {
                dimension: 0,
                runtime_id: 42,
                metadata: Arc::from([ActorMetadata {
                    key: 0,
                    value: ActorMetadataValue::Flags(1 << 1),
                }]),
                properties: Arc::from([]),
                tick: 2,
            })),
        )
        .unwrap();
    world.advance_actor_interpolation_ticks(2);
    let root = bone(&world, &entities, "root");
    let angle = 2.0 * root.rotation[3].clamp(-1.0, 1.0).acos().to_degrees();
    assert!((angle - 28.0).abs() < 1.0e-3, "root tilt {angle}");
}
