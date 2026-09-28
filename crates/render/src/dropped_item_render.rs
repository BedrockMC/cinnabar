//! Draws dropped-item sprite meshes in the opaque 3D phase with per-item lighting.
use crate::dropped_item::{
    DroppedItemScene, ITEM_MESH_VERTEX_BYTES, ItemMeshVertex, MAX_DROPPED_ITEM_INSTANCES,
    MAX_ITEM_SPRITE_SIDE, MAX_ITEM_SPRITES, extruded_sprite_mesh,
};
use bevy::{
    asset::{AssetId, load_internal_asset, uuid_handle},
    core_pipeline::core_3d::{CORE_3D_DEPTH_FORMAT, Opaque3d, Opaque3dBatchSetKey, Opaque3dBinKey},
    ecs::{
        change_detection::Tick,
        query::ROQueryItem,
        system::{SystemParam, SystemParamItem, lifetimeless::Read, lifetimeless::SRes},
    },
    mesh::VertexBufferLayout,
    prelude::*,
    render::{
        Render, RenderApp, RenderStartup, RenderSystems,
        extract_resource::ExtractResourcePlugin,
        render_phase::{
            AddRenderCommand, BinnedRenderPhaseType, DrawFunctions, InputUniformIndex, PhaseItem,
            RenderCommand, RenderCommandResult, SetItemPipeline, TrackedRenderPass,
            ViewBinnedRenderPhases,
        },
        render_resource::{
            AddressMode, BindGroup, BindGroupEntry, BindGroupLayoutDescriptor,
            BindGroupLayoutEntry, BindingResource, BindingType, Buffer, BufferBindingType,
            BufferDescriptor, BufferId, BufferInitDescriptor, BufferSize, BufferUsages, Canonical,
            ColorTargetState, ColorWrites, CompareFunction, DepthStencilState, Extent3d,
            FilterMode, FragmentState, PipelineCache, RenderPipeline, RenderPipelineDescriptor,
            Sampler, SamplerBindingType, SamplerDescriptor, ShaderStages, ShaderType, Specializer,
            SpecializerKey, Texture, TextureDataOrder, TextureDescriptor, TextureDimension,
            TextureFormat, TextureSampleType, TextureUsages, TextureView, TextureViewDescriptor,
            TextureViewDimension, Variants, VertexAttribute, VertexFormat, VertexState,
            VertexStepMode,
        },
        renderer::{RenderDevice, RenderQueue},
        sync_world::MainEntity,
        view::{ExtractedView, ViewTarget, ViewUniform, ViewUniformOffset, ViewUniforms},
    },
};
use std::ops::Range;

const ITEM_SHADER_HANDLE: Handle<Shader> = uuid_handle!("6b7c1b0e-2f3d-4a61-9d1e-7a8f2c4e5b13");

#[derive(Debug, Clone, Copy, Default)]
pub struct DroppedItemRenderPlugin;

impl Plugin for DroppedItemRenderPlugin {
    fn build(&self, app: &mut App) {
        install(app);
    }

    fn finish(&self, app: &mut App) {
        install(app);
    }
}

#[derive(Resource)]
struct Installed;

fn install(app: &mut App) {
    app.init_resource::<DroppedItemScene>();
    let Some(render_app) = app.get_sub_app(RenderApp) else {
        return;
    };
    if render_app.world().contains_resource::<Installed>() {
        return;
    }
    app.add_plugins(ExtractResourcePlugin::<DroppedItemScene>::default());
    load_internal_asset!(
        app,
        ITEM_SHADER_HANDLE,
        "dropped_item.wgsl",
        Shader::from_wgsl
    );
    app.sub_app_mut(RenderApp)
        .insert_resource(Installed)
        .init_resource::<ItemPipeline>()
        .add_render_command::<Opaque3d, DrawItemCommands>()
        .add_systems(RenderStartup, init_gpu)
        .add_systems(
            Render,
            (
                prepare_items.in_set(RenderSystems::PrepareResources),
                prepare_bind_group.in_set(RenderSystems::PrepareBindGroups),
                queue_items.in_set(RenderSystems::Queue),
            ),
        );
}

/// Per-copy vertex-rate data: three affine rows, then sprite layer and light levels.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct GpuItemInstance {
    rows: [[f32; 4]; 3],
    meta: [u32; 4],
}

const _: () = assert!(size_of::<GpuItemInstance>() == 64);

#[derive(Resource)]
struct ItemGpu {
    sampler: Sampler,
    environment: Buffer,
    instance_buffer: Buffer,
    mesh_buffer: Option<Buffer>,
    /// Vertex range of each sprite's mesh; empty for rejected sprites.
    ranges: Vec<Range<u32>>,
    _atlas: Option<Texture>,
    atlas_view: Option<TextureView>,
    sprites_revision: u64,
    /// This frame's `(vertex range, instance index)` draws.
    draws: Vec<(Range<u32>, u32)>,
    bind_group: Option<BindGroup>,
    view_buffer_id: Option<BufferId>,
}

fn init_gpu(mut commands: Commands, device: Res<RenderDevice>) {
    commands.insert_resource(ItemGpu {
        sampler: device.create_sampler(&SamplerDescriptor {
            label: Some("nearest dropped item sampler"),
            address_mode_u: AddressMode::ClampToEdge,
            address_mode_v: AddressMode::ClampToEdge,
            address_mode_w: AddressMode::ClampToEdge,
            mag_filter: FilterMode::Nearest,
            min_filter: FilterMode::Nearest,
            mipmap_filter: FilterMode::Nearest,
            ..default()
        }),
        environment: device.create_buffer(&BufferDescriptor {
            label: Some("dropped item environment"),
            size: 16,
            usage: BufferUsages::UNIFORM | BufferUsages::COPY_DST,
            mapped_at_creation: false,
        }),
        instance_buffer: device.create_buffer(&BufferDescriptor {
            label: Some("bounded dropped item instances"),
            size: (MAX_DROPPED_ITEM_INSTANCES * size_of::<GpuItemInstance>()) as u64,
            usage: BufferUsages::VERTEX | BufferUsages::COPY_DST,
            mapped_at_creation: false,
        }),
        mesh_buffer: None,
        ranges: Vec::new(),
        _atlas: None,
        atlas_view: None,
        sprites_revision: u64::MAX,
        draws: Vec::new(),
        bind_group: None,
        view_buffer_id: None,
    });
}

fn rebuild_sprites(
    scene: &DroppedItemScene,
    device: &RenderDevice,
    queue: &RenderQueue,
    gpu: &mut ItemGpu,
) {
    let side = MAX_ITEM_SPRITE_SIDE as usize;
    let layers = scene.sprites.len().clamp(1, MAX_ITEM_SPRITES);
    let mut atlas = vec![0_u8; layers * side * side * 4];
    let mut vertices: Vec<ItemMeshVertex> = Vec::new();
    let mut ranges = Vec::with_capacity(scene.sprites.len());
    for (layer, sprite) in scene.sprites.iter().take(MAX_ITEM_SPRITES).enumerate() {
        let start = vertices.len() as u32;
        if let Some(mesh) = extruded_sprite_mesh(
            sprite.width,
            sprite.height,
            &sprite.rgba8,
            MAX_ITEM_SPRITE_SIDE,
        ) {
            let row_bytes = sprite.width as usize * 4;
            for row in 0..sprite.height as usize {
                let source = &sprite.rgba8[row * row_bytes..(row + 1) * row_bytes];
                let target = (layer * side + row) * side * 4;
                atlas[target..target + row_bytes].copy_from_slice(source);
            }
            vertices.extend(mesh);
        }
        ranges.push(start..vertices.len() as u32);
    }
    gpu.mesh_buffer = (!vertices.is_empty()).then(|| {
        device.create_buffer_with_data(&BufferInitDescriptor {
            label: Some("dropped item sprite meshes"),
            contents: bytemuck::cast_slice::<ItemMeshVertex, u8>(&vertices),
            usage: BufferUsages::VERTEX,
        })
    });
    gpu.ranges = ranges;
    let texture = device.create_texture_with_data(
        queue,
        &TextureDescriptor {
            label: Some("dropped item sprite layers"),
            size: Extent3d {
                width: MAX_ITEM_SPRITE_SIDE,
                height: MAX_ITEM_SPRITE_SIDE,
                depth_or_array_layers: layers as u32,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: TextureDimension::D2,
            format: TextureFormat::Rgba8UnormSrgb,
            usage: TextureUsages::TEXTURE_BINDING | TextureUsages::COPY_DST,
            view_formats: &[],
        },
        TextureDataOrder::LayerMajor,
        &atlas,
    );
    gpu.atlas_view = Some(texture.create_view(&TextureViewDescriptor {
        label: Some("dropped item sprite layer array"),
        dimension: Some(TextureViewDimension::D2Array),
        ..default()
    }));
    gpu._atlas = Some(texture);
    gpu.sprites_revision = scene.sprites_revision;
    gpu.bind_group = None;
}

fn prepare_items(
    scene: Res<DroppedItemScene>,
    device: Res<RenderDevice>,
    queue: Res<RenderQueue>,
    mut gpu: ResMut<ItemGpu>,
) {
    if gpu.sprites_revision != scene.sprites_revision || gpu.atlas_view.is_none() {
        rebuild_sprites(&scene, &device, &queue, &mut gpu);
    }
    let mut instances = Vec::with_capacity(scene.instances.len());
    let mut draws = Vec::with_capacity(scene.instances.len());
    for instance in scene.instances.iter() {
        let Some(range) = gpu
            .ranges
            .get(instance.sprite as usize)
            .filter(|range| !range.is_empty())
        else {
            continue;
        };
        draws.push((range.clone(), instances.len() as u32));
        instances.push(GpuItemInstance {
            rows: instance.world_from_item,
            meta: [instance.sprite, instance.block_level, instance.sky_level, 0],
        });
    }
    if !instances.is_empty() {
        queue.write_buffer(
            &gpu.instance_buffer,
            0,
            bytemuck::cast_slice::<GpuItemInstance, u8>(&instances),
        );
    }
    gpu.draws = draws;
    queue.write_buffer(
        &gpu.environment,
        0,
        bytemuck::cast_slice::<f32, u8>(&[scene.daylight, 0.0, 0.0, 0.0]),
    );
}

struct ItemPipelineSpecializer;

#[derive(Resource)]
struct ItemPipeline {
    variants: Variants<RenderPipeline, ItemPipelineSpecializer>,
    bind_group_layout: BindGroupLayoutDescriptor,
}

impl FromWorld for ItemPipeline {
    fn from_world(_world: &mut World) -> Self {
        let bind_group_layout = item_bind_group_layout();
        let descriptor = item_pipeline_descriptor(bind_group_layout.clone());
        Self {
            variants: Variants::new(ItemPipelineSpecializer, descriptor),
            bind_group_layout,
        }
    }
}

fn item_bind_group_layout() -> BindGroupLayoutDescriptor {
    BindGroupLayoutDescriptor::new(
        "dropped item bind group layout",
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
                visibility: ShaderStages::FRAGMENT,
                ty: BindingType::Texture {
                    sample_type: TextureSampleType::Float { filterable: true },
                    view_dimension: TextureViewDimension::D2Array,
                    multisampled: false,
                },
                count: None,
            },
            BindGroupLayoutEntry {
                binding: 2,
                visibility: ShaderStages::FRAGMENT,
                ty: BindingType::Sampler(SamplerBindingType::Filtering),
                count: None,
            },
            BindGroupLayoutEntry {
                binding: 3,
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

fn item_pipeline_descriptor(layout: BindGroupLayoutDescriptor) -> RenderPipelineDescriptor {
    RenderPipelineDescriptor {
        label: Some("dropped item pipeline".into()),
        layout: vec![layout],
        vertex: VertexState {
            shader: ITEM_SHADER_HANDLE,
            entry_point: Some("item_vertex".into()),
            buffers: vec![
                VertexBufferLayout {
                    array_stride: ITEM_MESH_VERTEX_BYTES as u64,
                    step_mode: VertexStepMode::Vertex,
                    attributes: vec![
                        VertexAttribute {
                            format: VertexFormat::Float32x3,
                            offset: 0,
                            shader_location: 0,
                        },
                        VertexAttribute {
                            format: VertexFormat::Float32x2,
                            offset: 12,
                            shader_location: 1,
                        },
                        VertexAttribute {
                            format: VertexFormat::Float32x3,
                            offset: 20,
                            shader_location: 2,
                        },
                    ],
                },
                VertexBufferLayout {
                    array_stride: size_of::<GpuItemInstance>() as u64,
                    step_mode: VertexStepMode::Instance,
                    attributes: vec![
                        VertexAttribute {
                            format: VertexFormat::Float32x4,
                            offset: 0,
                            shader_location: 3,
                        },
                        VertexAttribute {
                            format: VertexFormat::Float32x4,
                            offset: 16,
                            shader_location: 4,
                        },
                        VertexAttribute {
                            format: VertexFormat::Float32x4,
                            offset: 32,
                            shader_location: 5,
                        },
                        VertexAttribute {
                            format: VertexFormat::Uint32x4,
                            offset: 48,
                            shader_location: 6,
                        },
                    ],
                },
            ],
            ..default()
        },
        fragment: Some(FragmentState {
            shader: ITEM_SHADER_HANDLE,
            entry_point: Some("item_fragment".into()),
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
struct ItemPipelineKey {
    msaa: Msaa,
    hdr: bool,
}

impl Specializer<RenderPipeline> for ItemPipelineSpecializer {
    type Key = ItemPipelineKey;

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
        Ok(key)
    }
}

fn prepare_bind_group(
    device: Res<RenderDevice>,
    cache: Res<PipelineCache>,
    pipeline: Res<ItemPipeline>,
    view_uniforms: Res<ViewUniforms>,
    mut gpu: ResMut<ItemGpu>,
) {
    let Some(view_binding) = view_uniforms.uniforms.binding() else {
        gpu.bind_group = None;
        return;
    };
    let Some(atlas_view) = gpu.atlas_view.as_ref() else {
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
    let bind_group = device.create_bind_group(
        "dropped item bind group",
        &cache.get_bind_group_layout(&pipeline.bind_group_layout),
        &[
            BindGroupEntry {
                binding: 0,
                resource: view_binding,
            },
            BindGroupEntry {
                binding: 1,
                resource: BindingResource::TextureView(atlas_view),
            },
            BindGroupEntry {
                binding: 2,
                resource: BindingResource::Sampler(&gpu.sampler),
            },
            BindGroupEntry {
                binding: 3,
                resource: gpu.environment.as_entire_binding(),
            },
        ],
    );
    gpu.bind_group = Some(bind_group);
    gpu.view_buffer_id = Some(view_buffer.id());
}

#[derive(SystemParam)]
struct QueueItemParams<'w, 's> {
    pipeline_cache: Res<'w, PipelineCache>,
    pipeline: ResMut<'w, ItemPipeline>,
    gpu: Res<'w, ItemGpu>,
    phases: ResMut<'w, ViewBinnedRenderPhases<Opaque3d>>,
    draw_functions: Res<'w, DrawFunctions<Opaque3d>>,
    views: Query<
        'w,
        's,
        (
            Entity,
            &'static MainEntity,
            &'static ExtractedView,
            &'static Msaa,
        ),
    >,
}

fn queue_items(mut params: QueueItemParams<'_, '_>, mut next_tick: Local<Tick>) {
    if params.gpu.draws.is_empty() || params.gpu.bind_group.is_none() {
        return;
    }
    let draw_function = params.draw_functions.read().id::<DrawItemCommands>();
    for (view_entity, main_entity, view, msaa) in &params.views {
        let Some(phase) = params.phases.get_mut(&view.retained_view_entity) else {
            continue;
        };
        let Ok(pipeline_id) = params.pipeline.variants.specialize(
            &params.pipeline_cache,
            ItemPipelineKey {
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

type DrawItemCommands = (SetItemPipeline, DrawItems);

struct DrawItems;

impl<P: PhaseItem> RenderCommand<P> for DrawItems {
    type Param = SRes<ItemGpu>;
    type ViewQuery = Read<ViewUniformOffset>;
    type ItemQuery = ();

    fn render<'w>(
        _item: &P,
        view: ROQueryItem<'w, '_, Self::ViewQuery>,
        _item_query: Option<ROQueryItem<'w, '_, Self::ItemQuery>>,
        gpu: SystemParamItem<'w, '_, Self::Param>,
        pass: &mut TrackedRenderPass<'w>,
    ) -> RenderCommandResult {
        let gpu = gpu.into_inner();
        let (Some(bind_group), Some(mesh)) = (gpu.bind_group.as_ref(), gpu.mesh_buffer.as_ref())
        else {
            return RenderCommandResult::Success;
        };
        pass.set_bind_group(0, bind_group, &[view.offset]);
        pass.set_vertex_buffer(0, mesh.slice(..));
        pass.set_vertex_buffer(1, gpu.instance_buffer.slice(..));
        for (range, instance) in &gpu.draws {
            pass.draw(range.clone(), *instance..*instance + 1);
        }
        RenderCommandResult::Success
    }
}
