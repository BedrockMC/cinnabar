use super::*;
use bevy::render::texture::CachedTexture;

/// Requests the format capabilities used by the live renderer without opening a window.
fn fixture() -> Option<(RenderDevice, wgpu::Queue, wgpu::Adapter)> {
    let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor::default());
    let adapter = match bevy::tasks::block_on(instance.request_adapter(&Default::default())) {
        Ok(adapter) => adapter,
        Err(wgpu::RequestAdapterError::NotFound { .. }) => {
            eprintln!("skipping scene sampling: missing native GPU fixture");
            return None;
        }
        Err(error) => panic!("scene sampling adapter: {error}"),
    };
    let (device, queue) = bevy::tasks::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        required_features: adapter.features()
            & wgpu::Features::TEXTURE_ADAPTER_SPECIFIC_FORMAT_FEATURES,
        ..Default::default()
    }))
    .unwrap();
    Some((RenderDevice::from(device), queue, adapter))
}

/// Creates the bounded fixture attachments with exactly their required usages.
fn texture(
    device: &RenderDevice,
    format: wgpu::TextureFormat,
    samples: u32,
    usage: wgpu::TextureUsages,
) -> Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some("scene sampling fixture"),
        size: wgpu::Extent3d {
            width: 2,
            height: 2,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: samples,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage,
        view_formats: &[],
    })
}

/// Reads one pixel after submitting the production commands.
fn pixel(
    device: &RenderDevice,
    queue: &wgpu::Queue,
    mut context: RenderContext,
    texture: &Texture,
) -> [u8; 4] {
    let readback = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("scene sampling readback"),
        size: 512,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    context.command_encoder().copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            aspect: if texture.format().is_depth_stencil_format() {
                wgpu::TextureAspect::DepthOnly
            } else {
                wgpu::TextureAspect::All
            },
            ..texture.as_image_copy()
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &readback,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(256),
                rows_per_image: Some(2),
            },
        },
        wgpu::Extent3d {
            width: 2,
            height: 2,
            depth_or_array_layers: 1,
        },
    );
    queue.submit(context.finish().0);
    readback
        .slice(..)
        .map_async(wgpu::MapMode::Read, |result| result.unwrap());
    device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
    readback.slice(..).get_mapped_range()[..4]
        .try_into()
        .unwrap()
}

#[test]
fn resolved_scene_copy_preserves_texels_for_each_supported_sample_count() {
    let Some((device, queue, adapter)) = fixture() else {
        return;
    };
    let format = wgpu::TextureFormat::Rgba8UnormSrgb;
    let source = texture(
        &device,
        format,
        1,
        wgpu::TextureUsages::COPY_DST | wgpu::TextureUsages::TEXTURE_BINDING,
    );
    let colours = [
        255, 0, 0, 255, 0, 0, 255, 255, 0, 255, 0, 255, 255, 255, 255, 255,
    ];
    queue.write_texture(
        source.as_image_copy(),
        &colours,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(8),
            rows_per_image: Some(2),
        },
        source.size(),
    );
    let source_view = source.create_view(&Default::default());
    for samples in [1, 2, 4, 8] {
        if !adapter
            .get_texture_format_features(format)
            .flags
            .sample_count_supported(samples)
        {
            eprintln!("skipping scene copy {samples} samples: missing format support");
            continue;
        }
        let destination = texture(
            &device,
            format,
            samples,
            wgpu::TextureUsages::RENDER_ATTACHMENT,
        );
        let destination_view = destination.create_view(&Default::default());
        let resolved = texture(
            &device,
            format,
            1,
            wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        );
        let resolved_view = resolved.create_view(&Default::default());
        let copy = SceneCopy::from_views(
            &device,
            [&source_view, &source_view],
            format,
            samples,
            RuntimeStage::GpuTransparent,
        );
        let mut context = RenderContext::new(device.clone(), None);
        if samples == 1 {
            copy.draw(
                &mut context,
                &World::new(),
                &source_view,
                &resolved_view,
                None,
                None,
            );
        } else {
            copy.draw(
                &mut context,
                &World::new(),
                &source_view,
                &destination_view,
                Some(&resolved_view),
                None,
            );
        }
        assert_eq!(pixel(&device, &queue, context, &resolved), [255, 0, 0, 255]);
    }
}

/// Writes distinct sample depths; one uncovered sample is the reverse-Z clear value.
fn fill_depth(context: &mut RenderContext, depth: &ViewDepthTexture, uncovered: bool) {
    let device = context.render_device().wgpu_device();
    let source = format!(
        "\
@vertex fn vertex(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {{
    return vec4(vec2(f32((i << 1u) & 2u), f32(i & 2u)) * 2.0 - vec2(1.0), 0.0, 1.0);
}}
@fragment fn fragment(@builtin(sample_index) sample: u32) -> @builtin(frag_depth) f32 {{
    return select(f32(sample + 1u) * 0.1, 0.0, {} && sample == 0u);
}}",
        uncovered
    );
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: None,
        source: wgpu::ShaderSource::Wgsl(source.into()),
    });
    let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("sample coverage fixture"),
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
            targets: &[],
        }),
        primitive: Default::default(),
        depth_stencil: Some(wgpu::DepthStencilState {
            format: wgpu::TextureFormat::Depth32Float,
            depth_write_enabled: true,
            depth_compare: wgpu::CompareFunction::Always,
            stencil: Default::default(),
            bias: Default::default(),
        }),
        multisample: wgpu::MultisampleState {
            count: depth.texture.sample_count(),
            ..Default::default()
        },
        multiview: None,
        cache: None,
    });
    let mut pass = context
        .command_encoder()
        .begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("sample coverage fixture"),
            color_attachments: &[],
            depth_stencil_attachment: Some(depth.get_attachment(wgpu::StoreOp::Store)),
            timestamp_writes: None,
            occlusion_query_set: None,
        });
    pass.set_pipeline(&pipeline);
    pass.draw(0..3, 0..1);
}

/// Runs the production Hi-Z seed directly on the same depth consumed by post effects.
fn hiz_seed(context: &mut RenderContext, depth: &ViewDepthTexture) -> Texture {
    let device = context.render_device();
    let destination = texture(
        device,
        wgpu::TextureFormat::R32Float,
        1,
        wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::COPY_SRC,
    );
    let view = destination.create_view(&Default::default());
    let device = device.wgpu_device();
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("production Hi-Z fixture"),
        source: wgpu::ShaderSource::Wgsl(include_str!("../chunk/gpu_cull/hiz.wgsl").into()),
    });
    let multi = depth.texture.sample_count() > 1;
    let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: None,
        layout: None,
        module: &shader,
        entry_point: Some(if multi {
            "hiz_seed_multisampled"
        } else {
            "hiz_seed"
        }),
        compilation_options: Default::default(),
        cache: None,
    });
    let binding = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: None,
        layout: &pipeline.get_bind_group_layout(0),
        entries: &[
            wgpu::BindGroupEntry {
                binding: u32::from(multi),
                resource: wgpu::BindingResource::TextureView(depth.view()),
            },
            wgpu::BindGroupEntry {
                binding: 3,
                resource: wgpu::BindingResource::TextureView(&view),
            },
        ],
    });
    let mut pass = context
        .command_encoder()
        .begin_compute_pass(&Default::default());
    pass.set_pipeline(&pipeline);
    pass.set_bind_group(0, &binding, &[]);
    pass.dispatch_workgroups(1, 1, 1);
    drop(pass);
    destination
}

#[test]
fn scene_depth_and_hiz_preserve_nearest_and_conservative_sample_coverage() {
    let Some((device, queue, adapter)) = fixture() else {
        return;
    };
    let format = wgpu::TextureFormat::Depth32Float;
    for samples in [1, 2, 4, 8] {
        if !adapter
            .get_texture_format_features(format)
            .flags
            .sample_count_supported(samples)
        {
            eprintln!("skipping depth resolve {samples} samples: missing format support");
            continue;
        }
        let texture = texture(
            &device,
            format,
            samples,
            wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
        );
        let depth = ViewDepthTexture::new(
            CachedTexture {
                default_view: texture.create_view(&Default::default()),
                texture,
            },
            Some(0.0),
        );
        let resolved = ResolvedDepth::new(&device, &depth, RuntimeStage::GpuPost);
        assert!(resolved.matches(&depth));
        for uncovered in [false, true] {
            let mut context = RenderContext::new(device.clone(), None);
            fill_depth(&mut context, &depth, uncovered);
            resolved.draw(&mut context, &World::new(), None);
            let expected = if samples == 1 && uncovered {
                0.0
            } else {
                samples as f32 * 0.1
            };
            assert!(
                (f32::from_le_bytes(pixel(&device, &queue, context, &resolved._texture))
                    - expected)
                    .abs()
                    < 1e-6
            );
            let mut context = RenderContext::new(device.clone(), None);
            let pyramid = hiz_seed(&mut context, &depth);
            let farthest = f32::from_le_bytes(pixel(&device, &queue, context, &pyramid));
            assert!((farthest - if uncovered { 0.0 } else { 0.1 }).abs() < 1e-6);
        }
    }
}

#[test]
fn scene_copy_and_depth_shaders_validate() {
    for source in [
        include_str!("../scene_copy.wgsl"),
        include_str!("../scene_depth.wgsl"),
    ] {
        let module = naga::front::wgsl::parse_str(source).unwrap();
        naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        )
        .validate(&module)
        .unwrap();
    }
}

#[test]
fn gamma_msaa_resolve_and_writeback_preserve_encoded_blending() {
    let Some((device, queue, adapter)) = fixture() else {
        return;
    };
    let linear = wgpu::TextureFormat::Rgba8UnormSrgb;
    let encoded = linear.remove_srgb_suffix();
    let source = texture(
        &device,
        linear,
        1,
        wgpu::TextureUsages::COPY_DST | wgpu::TextureUsages::TEXTURE_BINDING,
    );
    queue.write_texture(
        source.as_image_copy(),
        &[0, 0, 255, 255].repeat(4),
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(8),
            rows_per_image: Some(2),
        },
        source.size(),
    );
    let source_view = source.create_view(&Default::default());
    let shader = device
        .wgpu_device()
        .create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("encoded transparency fixture"),
            source: wgpu::ShaderSource::Wgsl(
                "\
@vertex fn vertex(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    return vec4(vec2(f32((i << 1u) & 2u), f32(i & 2u)) * 2.0 - vec2(1.0), 0.0, 1.0);
}
@fragment fn fragment() -> @location(0) vec4<f32> { return vec4(0.0, 1.0, 0.0, 0.25); }"
                    .into(),
            ),
        });
    for samples in [1, 2, 4, 8] {
        if !adapter
            .get_texture_format_features(encoded)
            .flags
            .sample_count_supported(samples)
        {
            eprintln!("skipping encoded resolve {samples} samples: missing format support");
            continue;
        }
        let descriptor = wgpu::TextureDescriptor {
            label: Some("encoded transparency fixture"),
            size: source.size(),
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: encoded,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::COPY_SRC
                | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[linear],
        };
        let resolved = device.create_texture(&descriptor);
        let resolved_encoded = resolved.create_view(&Default::default());
        let resolved_linear = resolved.create_view(&wgpu::TextureViewDescriptor {
            format: Some(linear),
            ..Default::default()
        });
        let sampled = device.create_texture(&wgpu::TextureDescriptor {
            sample_count: samples,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            ..descriptor
        });
        let sampled_encoded = sampled.create_view(&Default::default());
        let sampled_linear = sampled.create_view(&wgpu::TextureViewDescriptor {
            format: Some(linear),
            ..Default::default()
        });
        let copy = SceneCopy::from_views(
            &device,
            [&source_view, &source_view],
            linear,
            samples,
            RuntimeStage::GpuTransparent,
        );
        let mut context = RenderContext::new(device.clone(), None);
        let colour = if samples == 1 {
            &resolved_encoded
        } else {
            &sampled_encoded
        };
        let resolve = (samples > 1).then_some(&resolved_encoded);
        copy.draw(
            &mut context,
            &World::new(),
            &source_view,
            if samples == 1 {
                &resolved_linear
            } else {
                &sampled_linear
            },
            None,
            None,
        );
        let pipeline =
            device
                .wgpu_device()
                .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                    label: Some("encoded transparency fixture"),
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
                        targets: &[Some(wgpu::ColorTargetState {
                            format: encoded,
                            blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                            write_mask: wgpu::ColorWrites::COLOR,
                        })],
                    }),
                    primitive: Default::default(),
                    depth_stencil: None,
                    multisample: wgpu::MultisampleState {
                        count: samples,
                        ..Default::default()
                    },
                    multiview: None,
                    cache: None,
                });
        {
            let mut pass =
                context
                    .command_encoder()
                    .begin_render_pass(&wgpu::RenderPassDescriptor {
                        label: Some("encoded transparency fixture"),
                        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                            view: colour,
                            depth_slice: None,
                            resolve_target: resolve.map(|view| &**view),
                            ops: wgpu::Operations {
                                load: wgpu::LoadOp::Load,
                                store: wgpu::StoreOp::Store,
                            },
                        })],
                        depth_stencil_attachment: None,
                        timestamp_writes: None,
                        occlusion_query_set: None,
                    });
            pass.set_pipeline(&pipeline);
            pass.draw(0..3, 0..1);
        }
        assert_eq!(
            pixel(&device, &queue, context, &resolved),
            [0, 64, 191, 255]
        );
        let copied = texture(
            &device,
            linear,
            1,
            wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        );
        let copied_view = copied.create_view(&Default::default());
        let writeback = SceneCopy::from_views(
            &device,
            [&resolved_linear, &resolved_linear],
            linear,
            samples,
            RuntimeStage::GpuTransparent,
        );
        let mut context = RenderContext::new(device.clone(), None);
        writeback.draw(
            &mut context,
            &World::new(),
            &resolved_linear,
            if samples == 1 {
                &copied_view
            } else {
                &sampled_linear
            },
            (samples > 1).then_some(&copied_view),
            None,
        );
        assert_eq!(pixel(&device, &queue, context, &copied), [0, 64, 191, 255]);
    }
}
