//! The wgpu pass: one atlas texture, two storage-buffered vertex lists (cutout models in
//! the opaque phase, blended overlays in the transparent phase).

use std::mem::size_of;

use bevy::{
    asset::{AssetId, load_internal_asset, uuid_handle},
    core_pipeline::core_3d::{
        CORE_3D_DEPTH_FORMAT, Opaque3d, Opaque3dBatchSetKey, Opaque3dBinKey, Transparent3d,
    },
    ecs::{
        change_detection::Tick,
        query::ROQueryItem,
        system::{SystemParamItem, lifetimeless::Read, lifetimeless::SRes},
    },
    prelude::*,
    render::{
        Render, RenderApp, RenderStartup, RenderSystems,
        extract_resource::ExtractResourcePlugin,
        render_phase::{
            AddRenderCommand, BinnedRenderPhaseType, DrawFunctions, InputUniformIndex, PhaseItem,
            PhaseItemExtraIndex, RenderCommand, RenderCommandResult, SetItemPipeline,
            TrackedRenderPass, ViewBinnedRenderPhases, ViewSortedRenderPhases,
        },
        render_resource::{
            AddressMode, BindGroup, BindGroupEntry, BindGroupLayoutDescriptor,
            BindGroupLayoutEntry, BindingResource, BindingType, BlendState, Buffer,
            BufferBindingType, BufferDescriptor, BufferId, BufferSize, BufferUsages, Canonical,
            ColorTargetState, ColorWrites, CompareFunction, DepthStencilState, Extent3d,
            FilterMode, FragmentState, Origin3d, PipelineCache, RenderPipeline,
            RenderPipelineDescriptor, Sampler, SamplerBindingType, SamplerDescriptor, ShaderStages,
            ShaderType, Specializer, SpecializerKey, TexelCopyBufferLayout, TexelCopyTextureInfo,
            Texture, TextureDescriptor, TextureDimension, TextureFormat, TextureSampleType,
            TextureUsages, TextureView, TextureViewDescriptor, TextureViewDimension, Variants,
            VertexState,
        },
        renderer::{RenderDevice, RenderQueue},
        sync_world::MainEntity,
        view::{ExtractedView, ViewTarget, ViewUniform, ViewUniformOffset, ViewUniforms},
    },
};

use super::{
    mesh::{BLOCK_ENTITY_VERTEX_WORDS, BlockEntityVertex},
    scene::{BlockEntityFrame, BlockEntityScene},
};

const SHADER_HANDLE: Handle<Shader> = uuid_handle!("6f0c1c1e-3b6d-4a7e-9b1e-2f4f8a1d5c33");
const VERTEX_BYTES: u64 = (BLOCK_ENTITY_VERTEX_WORDS * size_of::<f32>()) as u64;
const MIN_BUFFER_VERTICES: u64 = 1024;

#[derive(Debug, Clone, Copy, Default)]
pub struct BlockEntityRenderPlugin;

impl Plugin for BlockEntityRenderPlugin {
    fn build(&self, app: &mut App) {
        install(app);
    }

    fn finish(&self, app: &mut App) {
        install(app);
    }
}

#[derive(Resource)]
struct BlockEntityRenderInstalled;

fn install(app: &mut App) {
    app.init_resource::<BlockEntityFrame>()
        .init_resource::<BlockEntityScene>();
    let Some(render_app) = app.get_sub_app(RenderApp) else {
        return;
    };
    if render_app
        .world()
        .contains_resource::<BlockEntityRenderInstalled>()
    {
        return;
    }
    app.add_plugins(ExtractResourcePlugin::<BlockEntityFrame>::default());
    load_internal_asset!(app, SHADER_HANDLE, "block_entity.wgsl", Shader::from_wgsl);
    app.sub_app_mut(RenderApp)
        .insert_resource(BlockEntityRenderInstalled)
        .init_resource::<BlockEntityPipeline>()
        .add_render_command::<Opaque3d, DrawSolidCommands>()
        .add_render_command::<Transparent3d, DrawOverlayCommands>()
        .add_systems(RenderStartup, init_gpu)
        .add_systems(
            Render,
            (
                prepare_resources.in_set(RenderSystems::PrepareResources),
                prepare_bind_groups.in_set(RenderSystems::PrepareBindGroups),
                (queue_solid, queue_overlay).in_set(RenderSystems::Queue),
            ),
        );
}

/// One storage buffer of vertices with its live count and bind group.
struct VertexList {
    buffer: Option<Buffer>,
    capacity_vertices: u64,
    count: u32,
    bind_group: Option<BindGroup>,
}

impl VertexList {
    const fn new() -> Self {
        Self {
            buffer: None,
            capacity_vertices: 0,
            count: 0,
            bind_group: None,
        }
    }

    fn upload(
        &mut self,
        vertices: &[BlockEntityVertex],
        render_device: &RenderDevice,
        render_queue: &RenderQueue,
        label: &'static str,
    ) {
        self.count = u32::try_from(vertices.len()).unwrap_or(0);
        if vertices.is_empty() {
            return;
        }
        let needed = vertices.len() as u64;
        if self.buffer.is_none() || self.capacity_vertices < needed {
            let capacity = needed.next_power_of_two().max(MIN_BUFFER_VERTICES);
            self.buffer = Some(render_device.create_buffer(&BufferDescriptor {
                label: Some(label),
                size: capacity * VERTEX_BYTES,
                usage: BufferUsages::STORAGE | BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }));
            self.capacity_vertices = capacity;
            self.bind_group = None;
        }
        if let Some(buffer) = &self.buffer {
            render_queue.write_buffer(buffer, 0, bytemuck::cast_slice(vertices));
        }
    }
}

#[derive(Resource)]
struct BlockEntityGpu {
    solid: VertexList,
    overlay: VertexList,
    texture: Option<Texture>,
    view: Option<TextureView>,
    atlas_identity: [u8; 32],
    atlas_size: [u32; 2],
    dynamic_revision: u64,
    sampler: Sampler,
    view_buffer_id: Option<BufferId>,
    frame_revision: u64,
}

fn init_gpu(mut commands: Commands, render_device: Res<RenderDevice>) {
    commands.insert_resource(BlockEntityGpu {
        solid: VertexList::new(),
        overlay: VertexList::new(),
        texture: None,
        view: None,
        atlas_identity: [0; 32],
        atlas_size: [0; 2],
        dynamic_revision: u64::MAX,
        sampler: render_device.create_sampler(&SamplerDescriptor {
            label: Some("block-entity nearest atlas sampler"),
            address_mode_u: AddressMode::ClampToEdge,
            address_mode_v: AddressMode::ClampToEdge,
            address_mode_w: AddressMode::ClampToEdge,
            mag_filter: FilterMode::Nearest,
            min_filter: FilterMode::Nearest,
            mipmap_filter: FilterMode::Nearest,
            ..default()
        }),
        view_buffer_id: None,
        frame_revision: u64::MAX,
    });
}

fn prepare_resources(
    frame: Res<BlockEntityFrame>,
    render_device: Res<RenderDevice>,
    render_queue: Res<RenderQueue>,
    mut gpu: ResMut<BlockEntityGpu>,
) {
    let Some(atlas) = frame.atlas.as_ref() else {
        gpu.solid.count = 0;
        gpu.overlay.count = 0;
        return;
    };
    if gpu.atlas_identity != atlas.identity || gpu.texture.is_none() {
        let texture = render_device.create_texture(&TextureDescriptor {
            label: Some("block-entity atlas"),
            size: Extent3d {
                width: atlas.size[0],
                height: atlas.size[1],
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: TextureDimension::D2,
            format: TextureFormat::Rgba8UnormSrgb,
            usage: TextureUsages::TEXTURE_BINDING | TextureUsages::COPY_DST,
            view_formats: &[],
        });
        write_rows(
            &render_queue,
            &texture,
            0,
            atlas.size[0],
            atlas.static_height,
            &atlas.static_rgba8,
        );
        gpu.view = Some(texture.create_view(&TextureViewDescriptor::default()));
        gpu.texture = Some(texture);
        gpu.atlas_identity = atlas.identity;
        gpu.atlas_size = atlas.size;
        gpu.dynamic_revision = u64::MAX;
        gpu.solid.bind_group = None;
        gpu.overlay.bind_group = None;
    }
    let dynamic_rows = atlas.size[1].saturating_sub(atlas.static_height);
    if gpu.dynamic_revision != frame.dynamic_revision
        && frame.dynamic_rgba8.len() == atlas.size[0] as usize * dynamic_rows as usize * 4
    {
        if let Some(texture) = &gpu.texture {
            write_rows(
                &render_queue,
                texture,
                atlas.static_height,
                atlas.size[0],
                dynamic_rows,
                &frame.dynamic_rgba8,
            );
        }
        gpu.dynamic_revision = frame.dynamic_revision;
    }
    if gpu.frame_revision != frame.revision {
        gpu.solid.upload(
            &frame.solid,
            &render_device,
            &render_queue,
            "block-entity solid vertices",
        );
        gpu.overlay.upload(
            &frame.overlay,
            &render_device,
            &render_queue,
            "block-entity overlay vertices",
        );
        gpu.frame_revision = frame.revision;
    }
}

fn write_rows(
    render_queue: &RenderQueue,
    texture: &Texture,
    first_row: u32,
    width: u32,
    rows: u32,
    rgba8: &[u8],
) {
    if rows == 0 {
        return;
    }
    render_queue.write_texture(
        TexelCopyTextureInfo {
            texture,
            mip_level: 0,
            origin: Origin3d {
                x: 0,
                y: first_row,
                z: 0,
            },
            aspect: default(),
        },
        rgba8,
        TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(width * 4),
            rows_per_image: Some(rows),
        },
        Extent3d {
            width,
            height: rows,
            depth_or_array_layers: 1,
        },
    );
}

struct BlockEntitySpecializer;

#[derive(Resource)]
struct BlockEntityPipeline {
    variants: Variants<RenderPipeline, BlockEntitySpecializer>,
    bind_group_layout: BindGroupLayoutDescriptor,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, SpecializerKey)]
struct BlockEntityPipelineKey {
    overlay: bool,
    msaa: Msaa,
    hdr: bool,
}

impl FromWorld for BlockEntityPipeline {
    fn from_world(_world: &mut World) -> Self {
        let bind_group_layout = BindGroupLayoutDescriptor::new(
            "block-entity bind group layout",
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
                        min_binding_size: BufferSize::new(VERTEX_BYTES),
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
            label: Some("block-entity pipeline".into()),
            layout: vec![bind_group_layout.clone()],
            vertex: VertexState {
                shader: SHADER_HANDLE,
                entry_point: Some("block_entity_vertex".into()),
                buffers: vec![],
                ..default()
            },
            fragment: Some(FragmentState {
                shader: SHADER_HANDLE,
                entry_point: Some("block_entity_solid".into()),
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
        };
        Self {
            variants: Variants::new(BlockEntitySpecializer, descriptor),
            bind_group_layout,
        }
    }
}

impl Specializer<RenderPipeline> for BlockEntitySpecializer {
    type Key = BlockEntityPipelineKey;

    fn specialize(
        &self,
        key: Self::Key,
        descriptor: &mut RenderPipelineDescriptor,
    ) -> Result<Canonical<Self::Key>, BevyError> {
        descriptor.multisample.count = key.msaa.samples();
        let fragment = descriptor.fragment.as_mut().unwrap();
        fragment.entry_point = Some(
            if key.overlay {
                "block_entity_overlay"
            } else {
                "block_entity_solid"
            }
            .into(),
        );
        let target = fragment.targets[0].as_mut().unwrap();
        target.format = if key.hdr {
            ViewTarget::TEXTURE_FORMAT_HDR
        } else {
            TextureFormat::bevy_default()
        };
        target.blend = key.overlay.then_some(BlendState::ALPHA_BLENDING);
        descriptor
            .depth_stencil
            .as_mut()
            .unwrap()
            .depth_write_enabled = !key.overlay;
        Ok(key)
    }
}

fn prepare_bind_groups(
    render_device: Res<RenderDevice>,
    pipeline_cache: Res<PipelineCache>,
    pipeline: Res<BlockEntityPipeline>,
    view_uniforms: Res<ViewUniforms>,
    mut gpu: ResMut<BlockEntityGpu>,
) {
    let Some(view_binding) = view_uniforms.uniforms.binding() else {
        gpu.solid.bind_group = None;
        gpu.overlay.bind_group = None;
        return;
    };
    let view_buffer = view_uniforms
        .uniforms
        .buffer()
        .expect("a dynamic view binding always owns a GPU buffer");
    if gpu.view_buffer_id != Some(view_buffer.id()) {
        gpu.solid.bind_group = None;
        gpu.overlay.bind_group = None;
        gpu.view_buffer_id = Some(view_buffer.id());
    }
    let BlockEntityGpu {
        solid,
        overlay,
        view,
        sampler,
        ..
    } = &mut *gpu;
    let Some(view) = view.as_ref() else {
        solid.bind_group = None;
        overlay.bind_group = None;
        return;
    };
    let layout = pipeline_cache.get_bind_group_layout(&pipeline.bind_group_layout);
    for (list, label) in [
        (solid, "block-entity solid bind group"),
        (overlay, "block-entity overlay bind group"),
    ] {
        let (Some(buffer), None) = (list.buffer.as_ref(), list.bind_group.as_ref()) else {
            continue;
        };
        list.bind_group = Some(render_device.create_bind_group(
            label,
            &layout,
            &[
                BindGroupEntry {
                    binding: 0,
                    resource: view_binding.clone(),
                },
                BindGroupEntry {
                    binding: 1,
                    resource: buffer.as_entire_binding(),
                },
                BindGroupEntry {
                    binding: 2,
                    resource: BindingResource::TextureView(view),
                },
                BindGroupEntry {
                    binding: 3,
                    resource: BindingResource::Sampler(sampler),
                },
            ],
        ));
    }
}

fn queue_solid(
    pipeline_cache: Res<PipelineCache>,
    mut pipeline: ResMut<BlockEntityPipeline>,
    gpu: Res<BlockEntityGpu>,
    mut phases: ResMut<ViewBinnedRenderPhases<Opaque3d>>,
    draw_functions: Res<DrawFunctions<Opaque3d>>,
    views: Query<(Entity, &MainEntity, &ExtractedView, &Msaa)>,
    mut next_tick: Local<Tick>,
) {
    if gpu.solid.count == 0 || gpu.solid.bind_group.is_none() {
        return;
    }
    let draw_function = draw_functions.read().id::<DrawSolidCommands>();
    for (view_entity, main_entity, view, msaa) in &views {
        let Some(phase) = phases.get_mut(&view.retained_view_entity) else {
            continue;
        };
        let Ok(pipeline_id) = pipeline.variants.specialize(
            &pipeline_cache,
            BlockEntityPipelineKey {
                overlay: false,
                msaa: *msaa,
                hdr: view.hdr,
            },
        ) else {
            continue;
        };
        let this_tick = next_tick.get() + 1;
        next_tick.set(this_tick);
        phase.add(
            Opaque3dBatchSetKey {
                draw_function,
                pipeline: pipeline_id,
                material_bind_group_index: None,
                lightmap_slab: None,
                vertex_slab: default(),
                index_slab: None,
            },
            Opaque3dBinKey {
                asset_id: AssetId::<Shader>::invalid().untyped(),
            },
            (view_entity, *main_entity),
            InputUniformIndex::default(),
            BinnedRenderPhaseType::NonMesh,
            *next_tick,
        );
    }
}

fn queue_overlay(
    pipeline_cache: Res<PipelineCache>,
    mut pipeline: ResMut<BlockEntityPipeline>,
    gpu: Res<BlockEntityGpu>,
    mut phases: ResMut<ViewSortedRenderPhases<Transparent3d>>,
    draw_functions: Res<DrawFunctions<Transparent3d>>,
    views: Query<(Entity, &MainEntity, &ExtractedView, &Msaa)>,
) {
    if gpu.overlay.count == 0 || gpu.overlay.bind_group.is_none() {
        return;
    }
    let draw_function = draw_functions.read().id::<DrawOverlayCommands>();
    for (view_entity, main_entity, view, msaa) in &views {
        let Some(phase) = phases.get_mut(&view.retained_view_entity) else {
            continue;
        };
        let Ok(pipeline_id) = pipeline.variants.specialize(
            &pipeline_cache,
            BlockEntityPipelineKey {
                overlay: true,
                msaa: *msaa,
                hdr: view.hdr,
            },
        ) else {
            continue;
        };
        phase.add(Transparent3d {
            entity: (view_entity, *main_entity),
            pipeline: pipeline_id,
            draw_function,
            // Overlays hug opaque geometry; drawing them last among blended items is enough.
            distance: 0.0,
            batch_range: 0..1,
            extra_index: PhaseItemExtraIndex::None,
            indexed: false,
        });
    }
}

type DrawSolidCommands = (SetItemPipeline, DrawList<false>);
type DrawOverlayCommands = (SetItemPipeline, DrawList<true>);

struct DrawList<const OVERLAY: bool>;

impl<P: PhaseItem, const OVERLAY: bool> RenderCommand<P> for DrawList<OVERLAY> {
    type Param = SRes<BlockEntityGpu>;
    type ViewQuery = Read<ViewUniformOffset>;
    type ItemQuery = ();

    fn render<'w>(
        _item: &P,
        view_offset: ROQueryItem<'w, '_, Self::ViewQuery>,
        _item_query: Option<ROQueryItem<'w, '_, Self::ItemQuery>>,
        gpu: SystemParamItem<'w, '_, Self::Param>,
        pass: &mut TrackedRenderPass<'w>,
    ) -> RenderCommandResult {
        let gpu = gpu.into_inner();
        let list = if OVERLAY { &gpu.overlay } else { &gpu.solid };
        let Some(bind_group) = &list.bind_group else {
            return RenderCommandResult::Skip;
        };
        if list.count == 0 {
            return RenderCommandResult::Skip;
        }
        pass.set_bind_group(0, bind_group, &[view_offset.offset]);
        pass.draw(0..list.count, 0..1);
        RenderCommandResult::Success
    }
}

#[cfg(test)]
const SHADER_SOURCE: &str = include_str!("block_entity.wgsl");

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shader_parses_and_reads_the_vertex_stride_the_cpu_writes() {
        let source = SHADER_SOURCE.replace(
            "#import bevy_render::view::View",
            "struct View { clip_from_world: mat4x4<f32>, }",
        );
        naga::front::wgsl::parse_str(&source).expect("block-entity shader parses");
        assert_eq!(BLOCK_ENTITY_VERTEX_WORDS, 9);
        assert_eq!(
            size_of::<BlockEntityVertex>(),
            BLOCK_ENTITY_VERTEX_WORDS * 4
        );
        assert!(source.contains("vertex_index * 9u"));
    }

    #[test]
    fn vertex_lists_track_counts_without_a_device() {
        let list = VertexList::new();
        assert_eq!(list.count, 0);
        assert!(list.buffer.is_none() && list.bind_group.is_none());
    }
}
