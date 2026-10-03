//! Item and artwork refreshes must retain unchanged actor geometry.

use std::sync::Arc;

use bevy::prelude::{Vec3, World};
use render::{ActorArtworkPages, ActorRenderFrame, ActorRenderScene};

use crate::runtime::{
    network::{
        HandRigBuilder,
        entity_pack::{SessionEntityPack, SessionItems},
        prepare_actor_render_frame, publish_actor_render_frame,
    },
    world::ClientWorld,
};

/// Publishes through the same two systems used by the native main frame.
fn publish(world: &mut World) -> ActorRenderFrame {
    world.run_system_cached(prepare_actor_render_frame).unwrap();
    world.run_system_cached(publish_actor_render_frame).unwrap();
    world.resource::<ActorRenderFrame>().clone()
}

/// Gives a compiled catalog its own immutable session identity.
fn session_pack(assets: Arc<assets::RuntimeEntityAssets>) -> Arc<SessionEntityPack> {
    Arc::new(SessionEntityPack {
        assets,
        textures: Arc::from([]),
        bindings: Arc::from([]),
        equipment: None,
    })
}

/// Installs one session into the production actor publication systems.
fn session_world(
    pack: Arc<SessionEntityPack>,
    artwork: ActorArtworkPages,
    scene: ActorRenderScene,
) -> World {
    let entities = pack.assets.clone();
    let mut client = ClientWorld::new_with_entity_assets(
        Arc::new(assets::RuntimeAssets::diagnostic()),
        entities.clone(),
    );
    client.stream = Some(super::actor_rest_presentation::stream(entities.clone()));
    client.pack_entities = Some(pack);
    super::actor_frame_allocations::actor_frame_world(
        client,
        scene,
        artwork,
        HandRigBuilder::from_runtime_assets(&entities).unwrap(),
        (Vec3::new(0.0, 66.0, -12.0), Vec3::new(0.0, 64.0, 4.0)),
    )
}

/// Refreshing item facts or artwork must not repack the entity and equipment namespaces.
#[test]
fn item_and_artwork_refreshes_retain_pack_geometry() {
    let (_fixture, artwork, entities) = super::actor_rest_presentation::compiled_fixture(
        "1.0",
        1,
        assets::ActorPoseMode::CompiledLiteral,
    );
    let scene = ActorRenderScene::with_runtime_entity_assets(&entities).unwrap();
    let mut world = session_world(session_pack(entities.clone()), artwork, scene);
    let original = publish(&mut world);
    assert!(
        world
            .resource::<ActorRenderScene>()
            .contains_geometry(render::pack_rig_id(0))
    );
    for artwork_changed in [false, true] {
        if artwork_changed {
            let artwork = world.resource::<ActorArtworkPages>().clone();
            *world.resource_mut::<ActorArtworkPages>() = artwork;
        } else {
            world.resource_mut::<ClientWorld>().session_items = Some(Arc::new(SessionItems {
                components: Arc::default(),
                icons: None,
            }));
        }
        let next = publish(&mut world);
        assert_eq!(next.rig.geometry_revision, original.rig.geometry_revision);
        assert_eq!(next.rig.geometry_spans, original.rig.geometry_spans);
        for span in next.rig.geometry_spans.iter() {
            assert_eq!(
                next.rig.geometry_vertices.span(*span),
                original.rig.geometry_vertices.span(*span),
            );
        }
        assert!(
            Arc::ptr_eq(
                &next.rig.geometry_vertices.segments,
                &original.rig.geometry_vertices.segments,
            ),
            "unchanged geometry was rebuilt after artwork_changed={artwork_changed}",
        );
    }
    world.resource_mut::<ClientWorld>().pack_entities = Some(session_pack(entities));
    let replacement = publish(&mut world);
    assert!(!Arc::ptr_eq(
        &replacement.rig.geometry_vertices.segments,
        &original.rig.geometry_vertices.segments,
    ));
    assert!(
        world
            .resource::<ActorRenderScene>()
            .contains_geometry(render::pack_rig_id(0))
    );
    world.resource_mut::<ClientWorld>().pack_entities = None;
    publish(&mut world);
    assert!(
        !world
            .resource::<ActorRenderScene>()
            .contains_geometry(render::pack_rig_id(0))
    );
}

/// One attachable uses the fixture's own geometry through the normal equipment compiler.
fn fixture_equipment() -> Arc<assets::RuntimeEquipmentCatalog> {
    let reference = |identifier: &str| assets::EquipmentReference {
        identifier: identifier.into(),
        resolution: assets::EntityDependencyResolution::Catalog,
    };
    Arc::new(
        assets::RuntimeEquipmentCatalog::from_parts(
            [1; 32],
            vec![assets::EquipmentBinding {
                identifier: "test:held".into(),
                category: assets::EquipmentCategory::Held,
                geometry: reference("geometry.example"),
                texture: reference("textures/entity/example"),
                material: "entity_alphatest".into(),
                render_controller: "controller.render.example".into(),
                first_person: assets::EquipmentTransform::NeedsMeasurement,
                third_person: assets::EquipmentTransform::NeedsMeasurement,
                dropped: assets::EquipmentTransform::NeedsMeasurement,
                poses: Box::new([]),
            }],
            Vec::new(),
        )
        .unwrap(),
    )
}

/// A rejected equipment range retries after capacity is freed without rebuilding accepted entities.
#[test]
fn rejected_equipment_retries_while_accepted_entities_stay_shared() {
    let (_fixture, artwork, entities) = super::actor_rest_presentation::compiled_fixture(
        "1.0",
        1,
        assets::ActorPoseMode::CompiledLiteral,
    );
    let mut pack = session_pack(entities.clone());
    Arc::get_mut(&mut pack).unwrap().equipment = Some(fixture_equipment());
    let equipment = render::pack_equipment_rig_id(
        render::find_geometry_index(&entities, "geometry.example").unwrap(),
    );
    let mut scene = ActorRenderScene::with_runtime_entity_assets(&entities).unwrap();
    scene.replace_pack_entities(Some(&entities)).unwrap();
    scene.reset();
    let filler_count = render::MAX_ACTOR_RIG_VERTICES - scene.frame().rig.geometry_vertices.len();
    scene.replace_pack_entities(None).unwrap();
    let filler_id = render::item_mesh_rig_id(0);
    let filler = |count| {
        render::ActorRigGeometry::new(
            filler_id,
            vec![render::ActorRigVertex::default(); count],
            vec![[0.0; 3]],
        )
        .unwrap()
    };
    scene.insert_geometry(filler(filler_count)).unwrap();
    let mut world = session_world(pack, artwork, scene);
    let original = publish(&mut world);
    assert!(
        world
            .resource::<ActorRenderScene>()
            .contains_geometry(render::pack_rig_id(0))
    );
    assert!(
        !world
            .resource::<ActorRenderScene>()
            .contains_geometry(equipment)
    );
    world.resource_mut::<ClientWorld>().session_items = Some(Arc::new(SessionItems {
        components: Arc::default(),
        icons: None,
    }));
    let retried = publish(&mut world);
    assert!(Arc::ptr_eq(
        &retried.rig.geometry_vertices.segments,
        &original.rig.geometry_vertices.segments,
    ));
    assert!(
        !world
            .resource::<ActorRenderScene>()
            .contains_geometry(equipment)
    );
    world
        .resource_mut::<ActorRenderScene>()
        .insert_geometry(filler(3))
        .unwrap();
    world.resource_mut::<ClientWorld>().session_items = Some(Arc::new(SessionItems {
        components: Arc::default(),
        icons: None,
    }));
    publish(&mut world);
    assert!(
        world
            .resource::<ActorRenderScene>()
            .contains_geometry(equipment)
    );
    assert!(
        world
            .resource::<ActorRenderScene>()
            .contains_geometry(render::pack_rig_id(0))
    );
}
