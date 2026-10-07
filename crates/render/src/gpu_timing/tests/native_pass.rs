use super::*;

#[test]
fn actual_render_pass_writes_readable_timestamps_across_reused_slots() {
    let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor::default());
    let adapter = match bevy::tasks::block_on(
        instance.request_adapter(&wgpu::RequestAdapterOptions::default()),
    ) {
        Ok(adapter) => adapter,
        Err(wgpu::RequestAdapterError::NotFound { .. }) => {
            eprintln!("skipping native render-pass timestamps: missing native GPU adapter fixture");
            return;
        }
        Err(error) => panic!("native timestamp adapter failed: {error}"),
    };
    if adapter.get_info().backend == wgpu::Backend::Noop
        || !adapter.features().contains(wgpu::Features::TIMESTAMP_QUERY)
    {
        eprintln!(
            "skipping native render-pass timestamps: missing timestamp-capable native GPU fixture"
        );
        return;
    }
    let (device, queue) = bevy::tasks::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        required_features: wgpu::Features::TIMESTAMP_QUERY,
        ..Default::default()
    }))
    .expect("native timestamp device");
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: None,
        source: wgpu::ShaderSource::Wgsl("@vertex fn vertex(@builtin(vertex_index) i: u32) -> @builtin(position) vec4f { let p = array(vec2f(-1,-1), vec2f(3,-1), vec2f(-1,3)); return vec4f(p[i],0,1); } @fragment fn fragment() -> @location(0) vec4f { return vec4f(1); }".into()),
    });
    let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: None,
        layout: None,
        vertex: wgpu::VertexState {
            module: &shader,
            entry_point: Some("vertex"),
            compilation_options: Default::default(),
            buffers: &[],
        },
        fragment: Some(wgpu::FragmentState {
            module: &shader,
            entry_point: Some("fragment"),
            compilation_options: Default::default(),
            targets: &[Some(wgpu::TextureFormat::Rgba8Unorm.into())],
        }),
        primitive: Default::default(),
        depth_stencil: None,
        multisample: Default::default(),
        multiview: None,
        cache: None,
    });
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: None,
        size: wgpu::Extent3d {
            width: 16,
            height: 16,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });
    let target = texture.create_view(&Default::default());
    let device = RenderDevice::from(device);
    let queue = RenderQueue(Arc::new(WgpuWrapper::new(queue)));
    let timestamps = GpuTimestamps::new(&device, &queue, false, wgpu::Backend::Metal).unwrap();
    let mut world = World::new();
    world.insert_resource(timestamps);
    let stages = [
        RuntimeStage::GpuTerrainSolid,
        RuntimeStage::GpuTerrainCutout,
        RuntimeStage::GpuTerrainModel,
        RuntimeStage::GpuOpaqueOther,
    ];
    let mut frames = Vec::new();
    let mut invalid_pairs = Vec::new();
    for cycle in 0..8 {
        for _ in 0..SLOTS {
            world
                .resource_mut::<GpuTimestamps>()
                .begin(|frame| frames.push(*frame));
            render_timestamped_categories(&world, &device, &queue, &target, &pipeline, &stages);
            world
                .resource_mut::<GpuTimestamps>()
                .submit(&device, &queue);
        }
        device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
        world
            .resource_mut::<GpuTimestamps>()
            .resolve_ready(&device, &queue);
        device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
        let mut timestamps = world.resource_mut::<GpuTimestamps>();
        invalid_pairs.extend(invalid_query_pairs(&timestamps, cycle));
        timestamps.begin(|frame| frames.push(*frame));
        timestamps.submit(&device, &queue);
        assert_eq!(frames.len(), (cycle + 1) * SLOTS);
    }
    for (index, frame) in frames.iter().enumerate() {
        for stage in stages {
            assert!(
                frame.get(stage).is_some(),
                "frame {index} is missing {stage:?}; raw invalid pairs: {invalid_pairs:?}"
            );
        }
        assert_eq!(frame.iter().count(), stages.len());
        assert!(frame.get(RuntimeStage::GpuFrame).is_none());
    }
}

/// Every category has real raster work and its own pass-boundary query pair.
fn render_timestamped_categories(
    world: &World,
    device: &RenderDevice,
    queue: &RenderQueue,
    target: &wgpu::TextureView,
    pipeline: &wgpu::RenderPipeline,
    stages: &[RuntimeStage],
) {
    let mut encoder = device.create_command_encoder(&Default::default());
    for stage in stages {
        let colors = [Some(wgpu::RenderPassColorAttachment {
            view: target,
            depth_slice: None,
            resolve_target: None,
            ops: wgpu::Operations {
                load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                store: wgpu::StoreOp::Store,
            },
        })];
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            timestamp_writes: pass_writes(world, *stage),
            color_attachments: &colors,
            ..Default::default()
        });
        pass.set_pipeline(pipeline);
        pass.draw(0..3, 0..1);
    }
    queue.submit([encoder.finish()]);
}

/// Captures invalid raw pairs before their mapped slots are drained, for assertion context.
fn invalid_query_pairs(
    timestamps: &GpuTimestamps,
    cycle: usize,
) -> Vec<(usize, usize, RuntimeStage, u64, u64)> {
    use super::super::{
        queries::MAPPED,
        readback::{SpanValidity, span_validity},
    };
    let mut invalid = Vec::new();
    for (index, slot) in timestamps.slots.iter().enumerate() {
        if slot.state.load(Ordering::Acquire) != MAPPED {
            continue;
        }
        let bytes = slot.buffer.slice(..).get_mapped_range();
        for span in 0..slot.passes {
            let offset = span as usize * 16;
            let begin = u64::from_le_bytes(bytes[offset..offset + 8].try_into().unwrap());
            let end = u64::from_le_bytes(bytes[offset + 8..offset + 16].try_into().unwrap());
            if span_validity(begin, end) != SpanValidity::Valid {
                invalid.push((cycle, index, slot.stages[span as usize], begin, end));
            }
        }
    }
    invalid
}
