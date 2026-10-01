use super::super::resource_sorts::ResourceView;
use super::*;
use bevy::render::renderer::WgpuWrapper;

/// A single transparent face exercises address preparation without external carriers.
fn water(tint: ChunkBiomeTintIdentity) -> ChunkRenderInstance {
    ChunkRenderInstance {
        key: SubChunkKey::new(0, 0, 0, 0),
        origin: [0; 3],
        generation: 1,
        cube_quads: Arc::from([]),
        cube_lighting: Arc::from([]),
        model_refs: Arc::from([]),
        model_lighting: Arc::from([]),
        model_draw_refs: Arc::from([]),
        transparent_model_draw_refs: Arc::from([]),
        liquid_quads: Arc::from([PackedLiquidQuad::try_pack(
            [0; 3],
            Face::PositiveY,
            [255; 4],
            0,
            0,
            [0; 2],
            false,
        )
        .unwrap()]),
        liquid_lighting: Arc::from([PackedQuadLighting::new([0; 4])]),
        has_depth_liquid: false,
        has_transparent_liquid: true,
        depth_liquid_start: None,
        biome: PackedBiomeRecord::fallback(),
        tint_identity: tint,
        priority: ChunkUploadPriority::new(0.0),
        token: None,
        publication_permit: None,
    }
}

#[derive(Resource)]
struct Candidate(Option<PreparedResourceGeometry>);

/// Calls the production publication boundary, including its deferred component writes.
fn publish(
    mut commands: Commands,
    instances: Query<(Entity, &ChunkRenderInstance)>,
    mut arena: ResMut<ChunkGpuArena>,
    mut candidate: ResMut<Candidate>,
) {
    candidate
        .0
        .take()
        .unwrap()
        .publish(&mut commands, &instances, &mut arena);
}

#[test]
fn publication_keeps_complete_transparent_addresses_and_biome_identity() {
    let (device, queue) = wgpu::Device::noop(&wgpu::DeviceDescriptor::default());
    let device = RenderDevice::from(device);
    let queue = RenderQueue(Arc::new(WgpuWrapper::new(queue)));
    let mut app = App::new();
    app.insert_resource(ChunkGpuArena::new(&device));
    let old_buffer = app
        .world()
        .resource::<ChunkGpuArena>()
        .geometry_stream_buffer
        .id();
    let view = app.world_mut().spawn_empty().id();
    let tint = ChunkBiomeTintIdentity::new(4, 7);
    let instance = water(tint);
    let entity = app.world_mut().spawn(instance.clone()).id();
    let assets = ChunkTextureAssets::default();
    let candidate = PreparedResourceGeometry::build(
        &[instance.clone()],
        assets.clone(),
        device.clone(),
        queue.clone(),
        Some(ResourceView {
            entity: view,
            transform: GlobalTransform::IDENTITY,
        }),
    )
    .unwrap();
    assert_eq!(
        app.world()
            .resource::<ChunkGpuArena>()
            .geometry_stream_buffer
            .id(),
        old_buffer
    );
    assert_eq!(candidate.liquids.state.committed().unwrap().refs().len(), 1);
    app.insert_resource(Candidate(Some(candidate)));
    app.world_mut().run_system_once(publish).unwrap();
    let arena = app.world().resource::<ChunkGpuArena>();
    assert_ne!(arena.geometry_stream_buffer.id(), old_buffer);
    assert_eq!(
        app.world()
            .get::<GpuChunkAllocation>(entity)
            .unwrap()
            .tint_identity,
        tint
    );
    let liquids = app.world().resource::<TransparentSortRuntime>();
    assert_eq!(liquids.view_entity, Some(view));
    assert!(transparent_snapshot_addresses_are_resident(
        liquids.state.committed().unwrap(),
        arena.allocations.values().map(|allocation| &allocation.gpu),
        std::iter::empty(),
        assets.identity(),
        tint
    ));
    let current = arena.geometry_stream_buffer.id();
    let mut malformed = instance;
    malformed.liquid_lighting = Arc::from([]);
    assert!(PreparedResourceGeometry::build(&[malformed], assets, device, queue, None).is_none());
    assert_eq!(
        app.world()
            .resource::<ChunkGpuArena>()
            .geometry_stream_buffer
            .id(),
        current
    );
}
