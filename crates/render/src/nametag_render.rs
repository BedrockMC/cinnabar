//! Draws [`NametagScene`] in the transparent 3D phase: a see-through pass over everything and
//! a depth-tested pass, as vanilla's `name_tag` and `name_tag_depth_tested` materials do.
use bevy::{
    asset::{load_internal_asset, uuid_handle},
    core_pipeline::core_3d::{CORE_3D_DEPTH_FORMAT, Transparent3d},
    ecs::{
        query::ROQueryItem,
        system::{SystemParamItem, lifetimeless::Read, lifetimeless::SRes},
    },
    prelude::*,
    render::{
        Render, RenderApp, RenderStartup, RenderSystems,
        extract_resource::ExtractResourcePlugin,
        render_phase::{
            AddRenderCommand, DrawFunctions, PhaseItem, PhaseItemExtraIndex, RenderCommand,
            RenderCommandResult, SetItemPipeline, TrackedRenderPass, ViewSortedRenderPhases,
        },
        render_resource::{
            AddressMode, BindGroup, BindGroupEntry, BindGroupLayoutDescriptor,
            BindGroupLayoutEntry, BindingResource, BindingType, BlendState, Buffer,
            BufferBindingType, BufferId, BufferInitDescriptor, BufferSize, BufferUsages, Canonical,
            ColorTargetState, ColorWrites, CompareFunction, DepthStencilState, Extent3d,
            FilterMode, FragmentState, PipelineCache, RenderPipeline, RenderPipelineDescriptor,
            Sampler, SamplerBindingType, SamplerDescriptor, ShaderStages, ShaderType, Specializer,
            SpecializerKey, TexelCopyBufferLayout, TextureAspect, TextureDescriptor,
            TextureDimension, TextureFormat, TextureSampleType, TextureUsages, TextureView,
            TextureViewDescriptor, TextureViewDimension, Variants, VertexState,
        },
        renderer::{RenderDevice, RenderQueue},
        sync_world::MainEntity,
        view::{ExtractedView, ViewTarget, ViewUniform, ViewUniformOffset, ViewUniforms},
    },
};

use crate::nametag::{MAX_NAMETAG_RECORDS, NAMETAG_ATLAS_SIDE, NametagRecord, NametagScene};

const NAMETAG_SHADER_HANDLE: Handle<Shader> = uuid_handle!("5d1f0c8e-2a47-4b93-9e6c-1f7a3b8d4c20");
const RECORD_BYTES: usize = std::mem::size_of::<NametagRecord>();

pub(crate) fn install_nametag_render(app: &mut App) {
    app.init_resource::<NametagScene>()
        .add_plugins(ExtractResourcePlugin::<NametagScene>::default());
    load_internal_asset!(
        app,
        NAMETAG_SHADER_HANDLE,
        "nametag.wgsl",
        Shader::from_wgsl
    );
    app.sub_app_mut(RenderApp)
        .init_resource::<NametagPipeline>()
        .add_render_command::<Transparent3d, DrawSeeThroughNametags>()
        .add_render_command::<Transparent3d, DrawDepthTestedNametags>()
        .add_systems(RenderStartup, init_nametag_gpu)
        .add_systems(
            Render,
            (
                prepare_nametags.in_set(RenderSystems::PrepareResources),
                prepare_nametag_bind_group.in_set(RenderSystems::PrepareBindGroups),
                queue_nametags
                    .run_if(crate::panorama::world_passes_enabled)
                    .in_set(RenderSystems::Queue),
            ),
        );
}

#[derive(Resource)]
struct NametagGpu {
    record_buffer: Buffer,
    atlas_view: TextureView,
    atlas_texture: bevy::render::render_resource::Texture,
    sampler: Sampler,
    atlas_revision: u64,
    see_through: u32,
    total: u32,
    bind_group: Option<BindGroup>,
    view_buffer_id: Option<BufferId>,
}

fn init_nametag_gpu(mut commands: Commands, render_device: Res<RenderDevice>) {
    let record_buffer = render_device.create_buffer_with_data(&BufferInitDescriptor {
        label: Some("nametag records"),
        contents: &vec![0_u8; MAX_NAMETAG_RECORDS * RECORD_BYTES],
        usage: BufferUsages::STORAGE | BufferUsages::COPY_DST,
    });
    let atlas_texture = render_device.create_texture(&TextureDescriptor {
        label: Some("nametag text atlas"),
        size: Extent3d {
            width: NAMETAG_ATLAS_SIDE,
            height: NAMETAG_ATLAS_SIDE,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: TextureDimension::D2,
        format: TextureFormat::Rgba8UnormSrgb,
        usage: TextureUsages::TEXTURE_BINDING | TextureUsages::COPY_DST,
        view_formats: &[],
    });
    let atlas_view = atlas_texture.create_view(&TextureViewDescriptor {
        dimension: Some(TextureViewDimension::D2),
        ..default()
    });
    let sampler = render_device.create_sampler(&SamplerDescriptor {
        label: Some("nametag atlas sampler"),
        address_mode_u: AddressMode::ClampToEdge,
        address_mode_v: AddressMode::ClampToEdge,
        mag_filter: FilterMode::Nearest,
        min_filter: FilterMode::Nearest,
        ..default()
    });
    commands.insert_resource(NametagGpu {
        record_buffer,
        atlas_view,
        atlas_texture,
        sampler,
        atlas_revision: 0,
        see_through: 0,
        total: 0,
        bind_group: None,
        view_buffer_id: None,
    });
}

fn prepare_nametags(
    scene: Res<NametagScene>,
    render_queue: Res<RenderQueue>,
    mut gpu: ResMut<NametagGpu>,
) {
    let total = scene.records.len().min(MAX_NAMETAG_RECORDS);
    gpu.total = total as u32;
    gpu.see_through = scene.see_through.min(total) as u32;
    if total > 0 {
        render_queue.write_buffer(
            &gpu.record_buffer,
            0,
            bytemuck::cast_slice::<NametagRecord, u8>(&scene.records[..total]),
        );
    }
    let side = NAMETAG_ATLAS_SIDE as usize;
    if scene.atlas_revision != gpu.atlas_revision && scene.atlas.len() == side * side * 4 {
        render_queue.write_texture(
            bevy::render::render_resource::TexelCopyTextureInfo {
                texture: &gpu.atlas_texture,
                mip_level: 0,
                origin: default(),
                aspect: TextureAspect::All,
            },
            &scene.atlas,
            TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(NAMETAG_ATLAS_SIDE * 4),
                rows_per_image: Some(NAMETAG_ATLAS_SIDE),
            },
            Extent3d {
                width: NAMETAG_ATLAS_SIDE,
                height: NAMETAG_ATLAS_SIDE,
                depth_or_array_layers: 1,
            },
        );
        gpu.atlas_revision = scene.atlas_revision;
    }
}

struct NametagPipelineSpecializer;

#[derive(Resource)]
struct NametagPipeline {
    variants: Variants<RenderPipeline, NametagPipelineSpecializer>,
    bind_group_layout: BindGroupLayoutDescriptor,
}

impl FromWorld for NametagPipeline {
    fn from_world(_world: &mut World) -> Self {
        let bind_group_layout = BindGroupLayoutDescriptor::new(
            "nametag bind group layout",
            &[
                BindGroupLayoutEntry {
                    binding: 0,
                    visibility: ShaderStages::VERTEX,
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
                        min_binding_size: BufferSize::new(RECORD_BYTES as u64),
                    },
                    count: None,
                },
                BindGroupLayoutEntry {
                    binding: 2,
                    visibility: ShaderStages::FRAGMENT,
                    ty: BindingType::Texture {
                        sample_type: TextureSampleType::Float { filterable: true },
                        view_dimension: TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                BindGroupLayoutEntry {
                    binding: 3,
                    visibility: ShaderStages::FRAGMENT,
                    ty: BindingType::Sampler(SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        );
        let descriptor = RenderPipelineDescriptor {
            label: Some("nametag pipeline".into()),
            layout: vec![bind_group_layout.clone()],
            vertex: VertexState {
                shader: NAMETAG_SHADER_HANDLE,
                entry_point: Some("nametag_vertex".into()),
                buffers: Vec::new(),
                ..default()
            },
            fragment: Some(FragmentState {
                shader: NAMETAG_SHADER_HANDLE,
                entry_point: Some("nametag_fragment".into()),
                targets: vec![Some(ColorTargetState {
                    format: TextureFormat::bevy_default(),
                    blend: Some(BlendState::ALPHA_BLENDING),
                    write_mask: ColorWrites::ALL,
                })],
                ..default()
            }),
            depth_stencil: Some(DepthStencilState {
                format: CORE_3D_DEPTH_FORMAT,
                depth_write_enabled: false,
                depth_compare: CompareFunction::Always,
                stencil: default(),
                bias: default(),
            }),
            ..default()
        };
        Self {
            variants: Variants::new(NametagPipelineSpecializer, descriptor),
            bind_group_layout,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, SpecializerKey)]
struct NametagPipelineKey {
    msaa: Msaa,
    hdr: bool,
    depth_tested: bool,
}

impl Specializer<RenderPipeline> for NametagPipelineSpecializer {
    type Key = NametagPipelineKey;

    fn specialize(
        &self,
        key: Self::Key,
        descriptor: &mut RenderPipelineDescriptor,
    ) -> Result<Canonical<Self::Key>, BevyError> {
        descriptor.multisample.count = key.msaa.samples();
        descriptor.fragment.as_mut().unwrap().targets[0]
            .as_mut()
            .unwrap()
            .format = if key.hdr {
            ViewTarget::TEXTURE_FORMAT_HDR
        } else {
            TextureFormat::bevy_default()
        };
        // Reverse-Z: nearer fragments carry larger depth.
        descriptor.depth_stencil.as_mut().unwrap().depth_compare = if key.depth_tested {
            CompareFunction::GreaterEqual
        } else {
            CompareFunction::Always
        };
        Ok(key)
    }
}

fn prepare_nametag_bind_group(
    render_device: Res<RenderDevice>,
    pipeline_cache: Res<PipelineCache>,
    pipeline: Res<NametagPipeline>,
    view_uniforms: Res<ViewUniforms>,
    mut gpu: ResMut<NametagGpu>,
) {
    let Some(view_binding) = view_uniforms.uniforms.binding() else {
        gpu.bind_group = None;
        return;
    };
    let view_buffer = view_uniforms
        .uniforms
        .buffer()
        .expect("a dynamic view binding always owns a GPU buffer");
    if gpu.bind_group.is_some() && gpu.view_buffer_id == Some(view_buffer.id()) {
        return;
    }
    let bind_group = render_device.create_bind_group(
        "nametag bind group",
        &pipeline_cache.get_bind_group_layout(&pipeline.bind_group_layout),
        &[
            BindGroupEntry {
                binding: 0,
                resource: view_binding,
            },
            BindGroupEntry {
                binding: 1,
                resource: gpu.record_buffer.as_entire_binding(),
            },
            BindGroupEntry {
                binding: 2,
                resource: BindingResource::TextureView(&gpu.atlas_view),
            },
            BindGroupEntry {
                binding: 3,
                resource: BindingResource::Sampler(&gpu.sampler),
            },
        ],
    );
    gpu.bind_group = Some(bind_group);
    gpu.view_buffer_id = Some(view_buffer.id());
}

fn queue_nametags(
    pipeline_cache: Res<PipelineCache>,
    mut pipeline: ResMut<NametagPipeline>,
    gpu: Res<NametagGpu>,
    mut phases: ResMut<ViewSortedRenderPhases<Transparent3d>>,
    draw_functions: Res<DrawFunctions<Transparent3d>>,
    views: Query<(Entity, &MainEntity, &ExtractedView, &Msaa)>,
) {
    if gpu.total == 0 {
        return;
    }
    let functions = draw_functions.read();
    // Distances grow toward the camera and sort ascending, so these draw after every other
    // transparent item, depth-tested tags first.
    let passes = [
        (
            true,
            gpu.see_through < gpu.total,
            functions.id::<DrawDepthTestedNametags>(),
            1.0e9,
        ),
        (
            false,
            gpu.see_through > 0,
            functions.id::<DrawSeeThroughNametags>(),
            2.0e9,
        ),
    ];
    for (view_entity, main_entity, view, msaa) in &views {
        let Some(phase) = phases.get_mut(&view.retained_view_entity) else {
            continue;
        };
        for (depth_tested, present, draw_function, distance) in passes {
            if !present {
                continue;
            }
            let Ok(pipeline_id) = pipeline.variants.specialize(
                &pipeline_cache,
                NametagPipelineKey {
                    msaa: *msaa,
                    hdr: view.hdr,
                    depth_tested,
                },
            ) else {
                continue;
            };
            phase.add(Transparent3d {
                entity: (view_entity, *main_entity),
                pipeline: pipeline_id,
                draw_function,
                distance,
                batch_range: 0..1,
                extra_index: PhaseItemExtraIndex::None,
                indexed: false,
            });
        }
    }
}

type DrawSeeThroughNametags = (
    SetItemPipeline,
    SetNametagBindGroup<0>,
    DrawNametagRange<false>,
);
type DrawDepthTestedNametags = (
    SetItemPipeline,
    SetNametagBindGroup<0>,
    DrawNametagRange<true>,
);

struct SetNametagBindGroup<const I: usize>;

impl<P: PhaseItem, const I: usize> RenderCommand<P> for SetNametagBindGroup<I> {
    type Param = SRes<NametagGpu>;
    type ViewQuery = Read<ViewUniformOffset>;
    type ItemQuery = ();

    fn render<'w>(
        _item: &P,
        view_offset: ROQueryItem<'w, '_, Self::ViewQuery>,
        _item_query: Option<ROQueryItem<'w, '_, Self::ItemQuery>>,
        gpu: SystemParamItem<'w, '_, Self::Param>,
        pass: &mut TrackedRenderPass<'w>,
    ) -> RenderCommandResult {
        let Some(bind_group) = &gpu.into_inner().bind_group else {
            return RenderCommandResult::Skip;
        };
        pass.set_bind_group(I, bind_group, &[view_offset.offset]);
        RenderCommandResult::Success
    }
}

struct DrawNametagRange<const DEPTH_TESTED: bool>;

impl<P: PhaseItem, const DEPTH_TESTED: bool> RenderCommand<P> for DrawNametagRange<DEPTH_TESTED> {
    type Param = SRes<NametagGpu>;
    type ViewQuery = ();
    type ItemQuery = ();

    fn render<'w>(
        _item: &P,
        _view: ROQueryItem<'w, '_, Self::ViewQuery>,
        _item_query: Option<ROQueryItem<'w, '_, Self::ItemQuery>>,
        gpu: SystemParamItem<'w, '_, Self::Param>,
        pass: &mut TrackedRenderPass<'w>,
    ) -> RenderCommandResult {
        let gpu = gpu.into_inner();
        let records = if DEPTH_TESTED {
            gpu.see_through..gpu.total
        } else {
            0..gpu.see_through
        };
        pass.draw(records.start * 6..records.end * 6, 0..1);
        RenderCommandResult::Success
    }
}

#[cfg(test)]
mod tests {
    use crate::nametag::{NAMETAG_BLOCKS_PER_FONT_PIXEL, NAMETAG_TEXT_LIFT_BLOCKS, NametagRecord};

    // The shader reads records at WGSL storage layout and repeats the tag scale and text lift.
    #[test]
    fn shader_mirrors_the_record_layout_scale_and_lift() {
        let source = include_str!("nametag.wgsl");
        assert_eq!(std::mem::size_of::<NametagRecord>(), 64);
        assert!(source.contains(&format!(
            "BLOCKS_PER_FONT_PIXEL: f32 = {NAMETAG_BLOCKS_PER_FONT_PIXEL};"
        )));
        assert!(source.contains(&format!(
            "TEXT_LIFT_BLOCKS: f32 = {NAMETAG_TEXT_LIFT_BLOCKS};"
        )));
    }
}
