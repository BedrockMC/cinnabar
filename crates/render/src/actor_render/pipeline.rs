use super::*;

pub(super) struct ActorPipelineSpecializer;

#[derive(Resource)]
pub(super) struct ActorPipeline {
    pub(super) variants: Variants<RenderPipeline, ActorPipelineSpecializer>,
    pub(super) bind_group_layout: BindGroupLayoutDescriptor,
    draw_variants: std::collections::HashMap<
        (Msaa, bool, u32),
        bevy::render::render_resource::CachedRenderPipelineId,
    >,
}

impl FromWorld for ActorPipeline {
    fn from_world(_world: &mut World) -> Self {
        let bind_group_layout = actor_bind_group_layout();
        let descriptor = actor_pipeline_descriptor(bind_group_layout.clone());
        Self {
            variants: Variants::new(ActorPipelineSpecializer, descriptor),
            bind_group_layout,
            draw_variants: Default::default(),
        }
    }
}

impl ActorPipeline {
    pub(super) fn prepare_draw_variants(
        &mut self,
        cache: &PipelineCache,
        msaa: Msaa,
        hdr: bool,
        materials: impl Iterator<Item = u32>,
    ) -> Option<bevy::render::render_resource::CachedRenderPipelineId> {
        for material in
            std::iter::once(assets::EntityRenderMaterial::Default as u32).chain(materials)
        {
            if self.draw_variants.contains_key(&(msaa, hdr, material)) {
                continue;
            }
            let id = self
                .variants
                .specialize(
                    cache,
                    ActorPipelineKey {
                        msaa,
                        hdr,
                        material,
                    },
                )
                .ok()?;
            self.draw_variants.insert((msaa, hdr, material), id);
        }
        self.draw_variants.get(&(msaa, hdr, 0)).copied()
    }

    pub(super) fn draw_variant(
        &self,
        msaa: Msaa,
        hdr: bool,
        material: u32,
    ) -> Option<bevy::render::render_resource::CachedRenderPipelineId> {
        self.draw_variants.get(&(msaa, hdr, material)).copied()
    }
}

pub(super) fn actor_bind_group_layout() -> BindGroupLayoutDescriptor {
    BindGroupLayoutDescriptor::new(
        "instanced actor bind group layout",
        &[
            BindGroupLayoutEntry {
                binding: 0,
                // The fragment stage reads the camera position for distance fog.
                visibility: ShaderStages::VERTEX | ShaderStages::FRAGMENT,
                ty: BindingType::Buffer {
                    ty: BufferBindingType::Uniform,
                    has_dynamic_offset: true,
                    min_binding_size: Some(ViewUniform::min_size()),
                },
                count: None,
            },
            BindGroupLayoutEntry {
                binding: 1,
                visibility: ShaderStages::VERTEX,
                ty: BindingType::Buffer {
                    ty: BufferBindingType::Storage { read_only: true },
                    has_dynamic_offset: false,
                    min_binding_size: BufferSize::new(size_of::<ActorGpuInstance>() as u64),
                },
                count: None,
            },
            BindGroupLayoutEntry {
                binding: 2,
                visibility: ShaderStages::VERTEX,
                ty: BindingType::Buffer {
                    ty: BufferBindingType::Storage { read_only: true },
                    has_dynamic_offset: false,
                    min_binding_size: BufferSize::new(size_of::<ActorRigVertex>() as u64),
                },
                count: None,
            },
            BindGroupLayoutEntry {
                binding: 3,
                visibility: ShaderStages::VERTEX,
                ty: BindingType::Buffer {
                    ty: BufferBindingType::Storage { read_only: true },
                    has_dynamic_offset: false,
                    min_binding_size: BufferSize::new(size_of::<ActorRigGeometrySpan>() as u64),
                },
                count: None,
            },
            BindGroupLayoutEntry {
                binding: 4,
                visibility: ShaderStages::VERTEX,
                ty: BindingType::Buffer {
                    ty: BufferBindingType::Storage { read_only: true },
                    has_dynamic_offset: false,
                    min_binding_size: BufferSize::new(size_of::<[[f32; 4]; 3]>() as u64),
                },
                count: None,
            },
            BindGroupLayoutEntry {
                binding: 5,
                visibility: ShaderStages::VERTEX,
                ty: BindingType::Buffer {
                    ty: BufferBindingType::Storage { read_only: true },
                    has_dynamic_offset: false,
                    min_binding_size: BufferSize::new(size_of::<[[f32; 4]; 3]>() as u64),
                },
                count: None,
            },
            BindGroupLayoutEntry {
                binding: 6,
                visibility: ShaderStages::FRAGMENT,
                ty: BindingType::Texture {
                    sample_type: TextureSampleType::Float { filterable: true },
                    view_dimension: TextureViewDimension::D2Array,
                    multisampled: false,
                },
                count: None,
            },
            BindGroupLayoutEntry {
                binding: 7,
                visibility: ShaderStages::FRAGMENT,
                ty: BindingType::Sampler(SamplerBindingType::Filtering),
                count: None,
            },
            BindGroupLayoutEntry {
                binding: 8,
                visibility: ShaderStages::FRAGMENT,
                ty: BindingType::Buffer {
                    ty: BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: BufferSize::new(16),
                },
                count: None,
            },
        ],
    )
}

pub(super) fn actor_pipeline_descriptor(
    bind_group_layout: BindGroupLayoutDescriptor,
) -> RenderPipelineDescriptor {
    RenderPipelineDescriptor {
        label: Some("bounded shared actor pipeline".into()),
        layout: vec![bind_group_layout, crate::lighting::layout()],
        vertex: VertexState {
            shader: ACTOR_SHADER_HANDLE,
            entry_point: Some("actor_vertex".into()),
            buffers: vec![],
            ..default()
        },
        fragment: Some(FragmentState {
            shader: ACTOR_SHADER_HANDLE,
            entry_point: Some("actor_fragment".into()),
            targets: vec![Some(ColorTargetState {
                format: TextureFormat::bevy_default(),
                blend: None,
                write_mask: ColorWrites::ALL,
            })],
            ..default()
        }),
        depth_stencil: Some(DepthStencilState {
            format: CORE_3D_DEPTH_FORMAT,
            depth_write_enabled: true,
            depth_compare: CompareFunction::GreaterEqual,
            stencil: default(),
            bias: default(),
        }),
        ..default()
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, SpecializerKey)]
pub(super) struct ActorPipelineKey {
    pub(super) msaa: Msaa,
    pub(super) hdr: bool,
    pub(super) material: u32,
}

impl Specializer<RenderPipeline> for ActorPipelineSpecializer {
    type Key = ActorPipelineKey;

    fn specialize(
        &self,
        key: Self::Key,
        descriptor: &mut RenderPipelineDescriptor,
    ) -> Result<Canonical<Self::Key>, BevyError> {
        descriptor.multisample.count = key.msaa.samples();
        if let Some(state) = crate::actor::material::state(key.material) {
            descriptor.primitive.cull_mode = state
                .cull
                .then_some(bevy::render::render_resource::Face::Back);
            descriptor
                .depth_stencil
                .as_mut()
                .unwrap()
                .depth_write_enabled = state.depth_write;
            if state.blend {
                let alpha = bevy::render::render_resource::BlendComponent {
                    src_factor: bevy::render::render_resource::BlendFactor::SrcAlpha,
                    dst_factor: bevy::render::render_resource::BlendFactor::OneMinusSrcAlpha,
                    operation: bevy::render::render_resource::BlendOperation::Add,
                };
                descriptor.fragment.as_mut().unwrap().targets[0]
                    .as_mut()
                    .unwrap()
                    .blend = Some(bevy::render::render_resource::BlendState {
                    color: alpha,
                    alpha,
                });
            }
        }
        let kind = key.material & assets::EntityRenderMaterialState::KIND_MASK;
        if kind == assets::EntityRenderMaterial::DissolveDepth as u32 {
            descriptor.fragment.as_mut().unwrap().targets[0]
                .as_mut()
                .unwrap()
                .write_mask = ColorWrites::empty();
        } else if kind == assets::EntityRenderMaterial::DissolveColor as u32 {
            descriptor.depth_stencil.as_mut().unwrap().depth_compare = CompareFunction::Equal;
        }
        descriptor.fragment.as_mut().unwrap().targets[0]
            .as_mut()
            .unwrap()
            .format = if key.hdr {
            ViewTarget::TEXTURE_FORMAT_HDR
        } else {
            TextureFormat::bevy_default()
        };
        Ok(key)
    }
}
