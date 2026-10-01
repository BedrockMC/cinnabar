//! Optional fixed-camera projectile frames through the actor scene publication path.
use std::{path::Path, sync::Arc};

use bevy::math::{Mat4, Vec3};
use protocol::{ActorEvent, ActorKind, ActorSpawnEvent, WorldEvent};

use super::{
    render_report::world_for,
    scene_report::{Frame, draw_actors},
};

/// Compiles the supplied vanilla pack and renders four fixed projectile states to scratch.
#[test]
fn render_projectile_states() {
    let (Ok(pack), Ok(out)) = (
        std::env::var("CINNABAR_PROJECTILE_PACK"),
        std::env::var("CINNABAR_PROJECTILE_OUT"),
    ) else {
        return;
    };
    let manifest = include_bytes!("../../../../../assets/vanilla-source.json");
    let compiled = asset_compiler::compile_entity_assets(Path::new(&pack), manifest).unwrap();
    let bytes = assets::encode_entity_blob(&compiled).unwrap();
    let entities = Arc::new(assets::RuntimeEntityAssets::decode(&bytes).unwrap());
    let artwork = asset_compiler::compile_actor_assets(Path::new(&pack), manifest).unwrap();
    let catalog = assets::RuntimeActorCatalog::decode(&artwork.bytes, &bytes).unwrap();
    let candidates = catalog
        .bindings()
        .iter()
        .map(|binding| binding.geometry_candidate)
        .collect::<Vec<_>>();
    let pages = render::ActorArtworkPages::default()
        .with_pack_artwork(catalog.textures(), catalog.bindings());
    std::fs::create_dir_all(&out).unwrap();
    for (name, identifier, pitch, yaw) in [
        ("arrow_flight", "minecraft:arrow", -20.0, 60.0),
        ("arrow_stuck", "minecraft:arrow", 0.0, 60.0),
        ("ender_pearl", "minecraft:ender_pearl", 0.0, 0.0),
        ("snowball", "minecraft:snowball", 0.0, 0.0),
    ] {
        let eye = Vec3::new(0.0, 0.25, 2.0);
        let mut world = world_for(&entities, &candidates, eye.to_array());
        world.set_actor_camera_rotation([0.0, 0.0]);
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
                    position: [0.0; 3],
                    velocity: [0.0; 3],
                    pitch,
                    yaw,
                    head_yaw: yaw,
                    body_yaw: yaw,
                    held_item: Default::default(),
                    metadata: Arc::from([]),
                    attributes: Arc::from([]),
                    properties: Arc::from([]),
                    links: Arc::from([]),
                })),
            )
            .unwrap();
        world.advance_actor_interpolation_ticks(1);
        let clip = Mat4::perspective_rh(45_f32.to_radians(), 1280.0 / 752.0, 0.05, 20.0)
            * Mat4::look_at_rh(eye, Vec3::new(0.0, 0.25, 0.0), Vec3::Y);
        let mut frame = Frame::new(clip);
        draw_actors(&mut frame, &world, &[42], &entities, &pages);
        frame
            .image
            .save(Path::new(&out).join(format!("{name}.png")))
            .unwrap();
    }
}
