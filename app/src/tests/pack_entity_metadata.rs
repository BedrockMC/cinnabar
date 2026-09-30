//! Independently authored server-pack entities driven by actor metadata end to end: the metadata
//! variant selects the render controller's texture and geometry, and the metadata scale sizes
//! the model.
use crate::presentation::{actors, entity_layers};
use assets::{RuntimeAssets, RuntimeEntityAssets};
use client_world::WorldStream;
use protocol::{
    ActorEvent, ActorKind, ActorMetadata, ActorMetadataUpdateEvent, ActorMetadataValue,
    ActorSpawnEvent, WorldBootstrap, WorldEvent,
};
use render::{ActorArtworkPages, EntityRigId};
use std::sync::Arc;

const COUNTER: &str = r#"{"format_version":"1.10.0","minecraft:client_entity":{"description":{
 "identifier":"test:counter",
 "materials":{"default":"entity_alphatest"},
 "textures":{"zero":"textures/entity/counter_zero","one":"textures/entity/counter_one"},
 "geometry":{"default":"geometry.counter"},
 "render_controllers":["controller.render.counter"]}}}"#;

// No `default` geometry alias: the controller picks the model, as display packs author it.
const HOLOGRAM: &str = r#"{"format_version":"1.10.0","minecraft:client_entity":{"description":{
 "identifier":"test:hologram",
 "materials":{"default":"entity_alphatest"},
 "textures":{"zero":"textures/entity/counter_zero","one":"textures/entity/counter_one"},
 "geometry":{"zero":"geometry.counter","one":"geometry.hologram_one"},
 "render_controllers":["controller.render.hologram"]}}}"#;

// A flipbook: `uv_anim` steps down a four-frame strip with the actor's life time.
const LOGO: &str = r#"{"format_version":"1.10.0","minecraft:client_entity":{"description":{
 "identifier":"test:logo",
 "materials":{"default":"entity_alphatest"},
 "textures":{"default":"textures/entity/counter_zero"},
 "geometry":{"default":"geometry.counter"},
 "render_controllers":["controller.render.logo"]}}}"#;

const GEOMETRY: &str = r#"{"format_version":"1.12.0","minecraft:geometry":[
 {"description":{"identifier":"geometry.counter","texture_width":16,"texture_height":16},"bones":[
 {"name":"root","pivot":[0,0,0],"cubes":[{"origin":[-4,0,-4],"size":[8,16,8],"uv":[0,0]}]}]},
 {"description":{"identifier":"geometry.hologram_one","texture_width":16,"texture_height":16},"bones":[
 {"name":"root","pivot":[0,0,0],"cubes":[{"origin":[-2,0,-2],"size":[4,24,4],"uv":[0,0]}]}]}]}"#;

const RENDER: &str = r#"{"format_version":"1.8.0","render_controllers":{
 "controller.render.counter":{
 "arrays":{"textures":{"Array.digits":["Texture.zero","Texture.one"]}},
 "geometry":"Geometry.default","materials":[{"*":"Material.default"}],
 "textures":["Array.digits[query.variant]"]},
 "controller.render.hologram":{
 "arrays":{"textures":{"Array.digits":["Texture.zero","Texture.one"]},
  "geometries":{"Array.models":["Geometry.zero","Geometry.one"]}},
 "geometry":"Array.models[query.variant]","materials":[{"*":"Material.default"}],
 "textures":["Array.digits[query.variant]"]},
 "controller.render.logo":{
 "geometry":"Geometry.default","materials":[{"*":"Material.default"}],
 "textures":["Texture.default"],
 "uv_anim":{"offset":[0.0,"math.mod(math.floor(query.life_time * 120), 4) / 4"],
  "scale":[1.0,"1 / 4"]}}}}"#;

fn png(colour: [u8; 4]) -> Vec<u8> {
    let mut bytes = Vec::new();
    image::RgbaImage::from_pixel(16, 16, image::Rgba(colour))
        .write_to(
            &mut std::io::Cursor::new(&mut bytes),
            image::ImageFormat::Png,
        )
        .unwrap();
    bytes
}

type Pack = (Arc<RuntimeEntityAssets>, Vec<u32>);

fn pack() -> (Pack, ActorArtworkPages) {
    let files: Vec<(Box<str>, Vec<u8>)> = vec![
        ("entity/counter.entity.json".into(), COUNTER.into()),
        ("entity/hologram.entity.json".into(), HOLOGRAM.into()),
        ("entity/logo.entity.json".into(), LOGO.into()),
        ("models/entity/counter.geo.json".into(), GEOMETRY.into()),
        (
            "render_controllers/counter.render_controllers.json".into(),
            RENDER.into(),
        ),
        (
            "textures/entity/counter_zero.png".into(),
            png([10, 0, 0, 255]),
        ),
        (
            "textures/entity/counter_one.png".into(),
            png([0, 10, 0, 255]),
        ),
    ];
    let compiled = asset_compiler::compile_actor_pack(files).unwrap().unwrap();
    let artwork =
        ActorArtworkPages::default().with_pack_artwork(&compiled.textures, &compiled.bindings);
    let candidates = compiled
        .bindings
        .iter()
        .map(|binding| binding.geometry_candidate)
        .collect();
    let assets = Arc::new(RuntimeEntityAssets::from_compiled(compiled.entities).unwrap());
    ((assets, candidates), artwork)
}

fn metadata(sequence: u64, key: u32, value: ActorMetadataValue) -> (u64, WorldEvent) {
    (
        sequence,
        WorldEvent::Actor(ActorEvent::Metadata(ActorMetadataUpdateEvent {
            dimension: 0,
            runtime_id: 42,
            metadata: Arc::from([ActorMetadata { key, value }]),
            properties: Arc::from([]),
            tick: sequence,
        })),
    )
}

fn world(pack: Pack, identifier: &str) -> WorldStream {
    let mut world = WorldStream::new_with_asset_sets(
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
        // The pack doubles as the base catalog; its layer still wins and yields pack rig ids.
        Arc::clone(&pack.0),
        [0.0, 64.0, 0.0],
        None,
    );
    world.set_pack_entities(Some(pack));
    world
        .submit(
            1,
            WorldEvent::Actor(ActorEvent::Spawn(ActorSpawnEvent {
                dimension: 0,
                unique_id: -42,
                runtime_id: 42,
                kind: ActorKind::Entity {
                    identifier: identifier.into(),
                },
                position: [0.0, 64.0, 0.0],
                velocity: [0.0; 3],
                pitch: 0.0,
                yaw: 0.0,
                head_yaw: 0.0,
                body_yaw: 0.0,
                held_item: Default::default(),
                metadata: Arc::from([ActorMetadata {
                    key: 2,
                    value: ActorMetadataValue::Int(0),
                }]),
                attributes: Arc::from([]),
                properties: Arc::from([]),
                links: Arc::from([]),
            })),
        )
        .unwrap();
    world.advance_actor_interpolation_ticks(2);
    world
}

fn update(world: &mut WorldStream, key: u32, value: ActorMetadataValue) {
    let (sequence, event) = metadata(2, key, value);
    world.submit(sequence, event).unwrap();
    world.advance_actor_interpolation_ticks(1);
}

struct Drawn {
    rig: EntityRigId,
    texture_layer: u32,
    model_scale: f32,
    /// Length of the drawn model's vertical axis, which the culling box follows.
    height_axis: f32,
    uv_anim: [f32; 4],
}

fn drawn(world: &WorldStream, artwork: &ActorArtworkPages) -> Drawn {
    let rig = world.actor_rig(42).unwrap();
    let body =
        actors::entity_rig_presentation(&rig, world.actor(42).unwrap(), artwork, 0.5).unwrap();
    let model_scale = body.model_scale;
    let mut batch = actors::select_actor_presentations(1, false, None, [body]);
    entity_layers::apply_render_layers(&mut batch, |id| world.actor_rig(id), artwork);
    let submission = &batch.submissions[0];
    let matrix = submission.world_from_actor;
    Drawn {
        rig: submission.input.rig,
        texture_layer: submission.texture_layer,
        model_scale,
        height_axis: (0..3).map(|row| matrix[row][1].powi(2)).sum::<f32>().sqrt(),
        uv_anim: submission.uv_anim,
    }
}

// A server-pack render controller picks its texture from the metadata variant every tick.
#[test]
fn metadata_variant_selects_the_pack_render_controller_texture() {
    let (pack, artwork) = pack();
    let mut world = world(pack, "test:counter");
    let before = drawn(&world, &artwork).texture_layer;
    update(&mut world, 2, ActorMetadataValue::Int(1));
    assert_ne!(
        before,
        drawn(&world, &artwork).texture_layer,
        "variant 1 draws the other texture layer"
    );
}

// Without a `default` alias the pack entity still gets a rig, and the variant swaps its model.
#[test]
fn metadata_variant_selects_the_pack_geometry_without_a_default_alias() {
    let (pack, artwork) = pack();
    let mut world = world(pack, "test:hologram");
    let zero = drawn(&world, &artwork);
    update(&mut world, 2, ActorMetadataValue::Int(1));
    let one = drawn(&world, &artwork);
    assert_ne!(zero.rig, one.rig, "variant 1 selects the other model");
    assert_ne!(zero.texture_layer, one.texture_layer);
}

// The metadata scale multiplies the model's size and its culling bounds.
#[test]
fn metadata_scale_multiplies_the_rendered_model() {
    let (pack, artwork) = pack();
    let mut world = world(pack, "test:counter");
    let unscaled = drawn(&world, &artwork);
    update(&mut world, 38, ActorMetadataValue::Float(2.0));
    let scaled = drawn(&world, &artwork);
    assert_eq!(scaled.model_scale, unscaled.model_scale * 2.0);
    assert!((scaled.height_axis - unscaled.height_axis * 2.0).abs() < 1e-5);
}

// `uv_anim` reaches the draw: the scale picks one frame and the offset follows the life time.
#[test]
fn render_controller_uv_anim_steps_the_flipbook_frame() {
    let (pack, artwork) = pack();
    let mut world = world(pack, "test:logo");
    let first = drawn(&world, &artwork).uv_anim;
    assert_eq!([first[0], first[2], first[3]], [0.0, 1.0, 0.25]);
    world.advance_actor_interpolation_ticks(1);
    let second = drawn(&world, &artwork).uv_anim;
    // Six frames pass per tick at 120 frames a second, two past a whole strip of four.
    assert_eq!((second[1] - first[1]).rem_euclid(1.0), 0.5);
}
