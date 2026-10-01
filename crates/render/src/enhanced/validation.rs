//! Validate every Enhanced shader and create its pipeline on an available real adapter.
#[path = "../../tests/support/shader_source.rs"]
mod shader_source;

type Variant = (&'static str, String, &'static str, &'static str, bool);

/// All forward, shadow-caster and fullscreen variants used by the extension.
fn variants() -> Vec<Variant> {
    let mut result = Vec::new();
    for (name, source) in [
        ("chunk", include_str!("../chunk.wgsl")),
        ("model", include_str!("../model.wgsl")),
        ("liquid", include_str!("../liquid.wgsl")),
    ] {
        result.push((
            name,
            shader_source::composed(source, &["ENHANCED"]),
            "vertex",
            "fragment",
            false,
        ));
        if name == "model" {
            result.push((
                name,
                shader_source::composed(source, &["ENHANCED"]),
                "vertex",
                "fragment_blend",
                false,
            ));
        }
        if name == "liquid" {
            result.push((
                name,
                shader_source::composed(source, &["ENHANCED"]),
                "vertex_depth",
                "fragment_depth",
                false,
            ));
        }
        if source.contains("#ifdef ENHANCED_SHADOW") {
            result.push((
                name,
                shader_source::composed(source, &["ENHANCED_SHADOW"]),
                "vertex",
                "fragment_shadow",
                true,
            ));
        }
    }
    for fragment in ["light_shafts", "composite"] {
        result.push((
            fragment,
            shader_source::composed(include_str!("../enhanced/post.wgsl"), &[]),
            "fullscreen",
            fragment,
            false,
        ));
    }
    result
}

#[test]
fn enhanced_shaders_validate() {
    for (name, source, _, fragment, _) in variants() {
        let module = naga::front::wgsl::parse_str(&source)
            .unwrap_or_else(|error| panic!("{name}/{fragment}: {}", error.emit_to_string(&source)));
        naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        )
        .validate(&module)
        .unwrap_or_else(|error| panic!("{name}/{fragment}: {error:?}"));
    }
}

#[test]
fn enhanced_pipelines_build_on_native_adapter() {
    let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor::default());
    let Ok(adapter) =
        bevy::tasks::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default()))
    else {
        assert!(
            std::env::var_os("CINNABAR_REQUIRE_ENHANCED_GPU").is_none(),
            "native Enhanced GPU validation was required but no adapter is available"
        );
        eprintln!("Enhanced GPU smoke skipped: no native adapter; Naga validation still runs");
        return;
    };
    let (device, _) = bevy::tasks::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some("enhanced smoke"),
        required_limits: adapter.limits(),
        ..Default::default()
    }))
    .expect("Enhanced smoke device");
    for (name, source, vertex, fragment, shadow) in variants() {
        let descriptors = if vertex == "fullscreen" {
            vec![super::gpu::enhanced_post_layout()]
        } else {
            vec![
                crate::chunk::enhanced::chunk_bind_group_layout(),
                if shadow {
                    super::gpu::enhanced_caster_layout()
                } else {
                    super::gpu::enhanced_view_layout()
                },
            ]
        };
        device.push_error_scope(wgpu::ErrorFilter::Validation);
        let groups: Vec<_> = descriptors
            .iter()
            .map(|descriptor| {
                device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                    label: Some(descriptor.label.as_ref()),
                    entries: &descriptor.entries,
                })
            })
            .collect();
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("production Enhanced layout"),
            bind_group_layouts: &groups.iter().collect::<Vec<_>>(),
            push_constant_ranges: &[],
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some(name),
            source: wgpu::ShaderSource::Wgsl(source.into()),
        });
        let targets = [Some(wgpu::ColorTargetState {
            format: wgpu::TextureFormat::Rgba16Float,
            blend: None,
            write_mask: wgpu::ColorWrites::ALL,
        })];
        let _pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some(name),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some(vertex),
                compilation_options: Default::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some(fragment),
                compilation_options: Default::default(),
                targets: if shadow { &[] } else { &targets },
            }),
            primitive: Default::default(),
            depth_stencil: shadow.then_some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: true,
                depth_compare: wgpu::CompareFunction::LessEqual,
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: Default::default(),
            multiview: None,
            cache: None,
        });
        let error = bevy::tasks::block_on(device.pop_error_scope());
        assert!(error.is_none(), "{name}/{fragment}: {error:?}");
    }
}
