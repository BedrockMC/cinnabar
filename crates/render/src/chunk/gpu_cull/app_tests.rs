//! The real render app on a validating NOOP device with count-driven indirect draws.

use bevy::{
    asset::{AssetPlugin, Assets},
    camera::{CameraPlugin, RenderTarget},
    core_pipeline::CorePipelinePlugin,
    image::ImagePlugin,
    mesh::MeshPlugin,
    render::{
        RenderPlugin,
        renderer::{RenderAdapterInfo, WgpuWrapper},
        settings::RenderCreation,
    },
    window::WindowPlugin,
};

use super::*;

fn noop_render_plugin() -> RenderPlugin {
    let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
        backends: wgpu::Backends::NOOP,
        backend_options: wgpu::BackendOptions {
            noop: wgpu::NoopBackendOptions { enable: true },
            ..Default::default()
        },
        ..Default::default()
    });
    let adapter =
        bevy::tasks::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default()))
            .unwrap();
    let descriptor = wgpu::DeviceDescriptor {
        required_features: WgpuFeatures::MULTI_DRAW_INDIRECT_COUNT
            | WgpuFeatures::INDIRECT_FIRST_INSTANCE,
        required_limits: wgpu::Limits {
            max_storage_buffers_per_shader_stage: required_vertex_storage_buffers(),
            ..Default::default()
        },
        ..Default::default()
    };
    let adapter_info = adapter.get_info();
    let (device, queue) = bevy::tasks::block_on(adapter.request_device(&descriptor)).unwrap();
    RenderPlugin {
        render_creation: RenderCreation::manual(
            RenderDevice::from(device),
            RenderQueue(Arc::new(WgpuWrapper::new(queue))),
            RenderAdapterInfo(WgpuWrapper::new(adapter_info)),
            RenderAdapter(Arc::new(WgpuWrapper::new(adapter))),
            RenderInstance(Arc::new(WgpuWrapper::new(instance))),
        ),
        synchronous_pipeline_compilation: true,
        ..Default::default()
    }
}

fn mesh() -> meshing::ChunkMesh {
    let source = world::SubChunk::decode(&[9, 1, 0, 1, 2], &world::RawBlockIds { air: 0 });
    meshing::mesh_sub_chunk(
        &meshing::BlockClassifier::new(0),
        &assets::RuntimeAssets::diagnostic(),
        assets::NetworkIdMode::Sequential,
        &meshing::Neighbourhood::empty(),
        &source,
    )
}

fn frame(app: &mut App) {
    app.update();
    app.sub_app(RenderApp)
        .world()
        .resource::<RenderDevice>()
        .poll(PollType::wait_indefinitely())
        .unwrap();
}

/// Every frame queues, prepares, culls and draws the GPU path without a validation error.
#[test]
fn count_capable_devices_run_the_two_phase_cull_through_the_render_graph() {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins)
        .add_plugins(WindowPlugin {
            primary_window: None,
            ..Default::default()
        })
        .add_plugins(AssetPlugin::default())
        .add_plugins(noop_render_plugin())
        .add_plugins((
            ImagePlugin::default(),
            MeshPlugin,
            CameraPlugin,
            CorePipelinePlugin,
        ))
        .add_plugins(ChunkRenderPlugin::default());
    let image = app
        .world_mut()
        .resource_mut::<Assets<Image>>()
        .add(Image::new_target_texture(
            64,
            48,
            TextureFormat::Rgba8Unorm,
            None,
        ));
    app.world_mut().spawn((
        Camera3d::default(),
        Camera::default(),
        RenderTarget::Image(image.into()),
        Msaa::Sample4,
        Transform::from_xyz(-6.0, 20.0, -6.0).looking_at(Vec3::new(8.0, 8.0, 8.0), Vec3::Y),
    ));
    app.finish();
    app.cleanup();
    let keys = [SubChunkKey::new(0, 0, 0, 0), SubChunkKey::new(0, 1, 0, 0)];
    for key in keys {
        app.world_mut()
            .resource_mut::<ChunkRenderQueue>()
            .try_insert(key, mesh(), ChunkUploadPriority::new(0.0))
            .unwrap();
    }
    for _ in 0..4 {
        frame(&mut app);
    }
    let render_world = app.sub_app(RenderApp).world();
    assert_eq!(
        render_world.resource::<GpuCullSupport>(),
        &GpuCullSupport(true)
    );
    let cull = render_world.resource::<GpuCull>();
    assert_eq!(cull.slot_count(), 2);
    assert!(cull.table.records().iter().all(CullRecord::is_live));
    assert!(
        model::slot_enabled(cull.table.enabled(), 0)
            && model::slot_enabled(cull.table.enabled(), 1)
    );
    assert!(cull.bind_groups.is_some(), "the culled view was prepared");
    assert!(
        cull.pyramid.is_some(),
        "the depth target admits sampling, so the late phase tests Hi-Z"
    );

    // Removing a chunk frees its slot on the next frame.
    let entity = app.world().resource::<ChunkEntities>().0[&keys[1]];
    app.world_mut()
        .entity_mut(entity)
        .remove::<ChunkRenderInstance>();
    frame(&mut app);
    frame(&mut app);
    let cull = app.sub_app(RenderApp).world().resource::<GpuCull>();
    assert_eq!(cull.slot_count(), 1);
}
