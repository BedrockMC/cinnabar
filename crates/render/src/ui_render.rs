use std::{
    mem::size_of,
    sync::{Arc, Weak},
};

use bevy::{
    asset::{load_internal_asset, uuid_handle},
    ecs::system::SystemChangeTick,
    mesh::VertexBufferLayout,
    prelude::*,
    render::{
        Render, RenderApp, RenderStartup, RenderSystems,
        extract_resource::ExtractResourcePlugin,
        render_resource::{
            AddressMode, BindGroupEntry, BindGroupLayoutDescriptor, BindGroupLayoutEntry,
            BindingResource, BindingType, BlendComponent, BlendFactor, BlendOperation, BlendState,
            Buffer, BufferBindingType, BufferDescriptor, BufferInitDescriptor, BufferSize,
            BufferUsages, CachedRenderPipelineId, Canonical, ColorTargetState, ColorWrites,
            FilterMode, FragmentState, PipelineCache, RenderPipeline, RenderPipelineDescriptor,
            Sampler, SamplerBindingType, SamplerDescriptor, ShaderStages, Specializer,
            SpecializerKey, TextureFormat, TextureSampleType, TextureViewDimension, Variants,
            VertexAttribute, VertexFormat, VertexState, VertexStepMode,
        },
        renderer::{RenderDevice, RenderQueue},
        sync_world::MainEntity,
        view::{ExtractedView, ViewTarget},
    },
};
use bytemuck::{Pod, Zeroable};

#[path = "ui_render/textures.rs"]
mod textures;
pub(crate) use textures::DeviceObservation;
use textures::UiGpuTextures;
#[path = "ui_render/overlay.rs"]
pub(crate) mod overlay;
#[path = "ui_render/uploads.rs"]
mod uploads;
use overlay::queue_ui_overlay;
pub(crate) use overlay::{UiHandCoverage, UiOverlayLabel, install_overlay_graph};

use crate::ui::{
    MAX_UI_INDICES, MAX_UI_VERTICES, UI_BLEND_INVERT, UiRenderBatch, UiRenderInput,
    UiRenderRejectReason, UiRenderScene, UiRenderStats, UiRenderVertex,
};
#[cfg(test)]
use crate::ui::{UiRenderReject, UiScissor};

const UI_SHADER_HANDLE: Handle<Shader> = uuid_handle!("7cfb904c-c8cf-4dd2-9214-7d208ce454e7");

#[derive(Debug, Clone, Copy, Default)]
pub struct UiRenderPlugin;

impl Plugin for UiRenderPlugin {
    fn build(&self, app: &mut App) {
        install_ui_render(app);
    }

    fn finish(&self, app: &mut App) {
        install_ui_render(app);
    }
}

#[derive(Resource)]
struct UiRenderInstalled;

fn install_ui_render(app: &mut App) {
    app.init_resource::<UiRenderScene>()
        .init_resource::<UiRenderStats>();
    let Some(render_app) = app.get_sub_app(RenderApp) else {
        return;
    };
    if render_app.world().contains_resource::<UiRenderInstalled>() {
        install_overlay_graph(app.sub_app_mut(RenderApp).world_mut());
        return;
    }
    let stats = app.world().resource::<UiRenderStats>().clone();
    app.add_plugins(ExtractResourcePlugin::<UiRenderScene>::default());
    load_internal_asset!(app, UI_SHADER_HANDLE, "ui.wgsl", Shader::from_wgsl);
    app.sub_app_mut(RenderApp)
        .insert_resource(UiRenderInstalled)
        .init_resource::<UiPipeline>()
        .insert_resource(stats)
        .init_resource::<UiHandCoverage>()
        .add_systems(RenderStartup, init_ui_gpu)
        .add_systems(
            Render,
            (
                prepare_ui_resources.in_set(RenderSystems::PrepareResources),
                prepare_ui_bind_group.in_set(RenderSystems::PrepareBindGroups),
                queue_ui_overlay.in_set(RenderSystems::Queue),
            ),
        );
    install_overlay_graph(app.sub_app_mut(RenderApp).world_mut());
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct UiViewportUniform {
    viewport_size: [f32; 2],
    /// Seconds since the UI renderer started; animates the item glint.
    time_seconds: f32,
    _padding: f32,
}

#[derive(Resource)]
pub(crate) struct UiGpu {
    device: wgpu::Device,
    device_observation: DeviceObservation,
    vertex_buffer: Option<Buffer>,
    index_buffer: Option<Buffer>,
    vertex_capacity: usize,
    index_capacity: usize,
    vertex_arena_id: u64,
    index_arena_id: u64,
    viewport_buffer: Buffer,
    viewport_size: [u32; 2],
    started: std::time::Instant,
    textures: UiGpuTextures,
    sampler: Sampler,
    batches: Arc<[UiRenderBatch]>,
    accepted_revision: Option<u64>,
    // Admission watermark survives every draw rejection, even after payload drop.
    last_admitted_revision: Option<u64>,
    last_admitted_publication: Weak<UiRenderInput>,
    index_count: usize,
    uploads: uploads::BufferUploads,
    view_pipelines:
        std::collections::BTreeMap<Entity, (CachedRenderPipelineId, CachedRenderPipelineId)>,
}

fn init_ui_gpu(mut commands: Commands, render_device: Res<RenderDevice>, tick: SystemChangeTick) {
    let viewport_buffer = render_device.create_buffer_with_data(&BufferInitDescriptor {
        label: Some("shared UI viewport uniform"),
        contents: bytemuck::bytes_of(&UiViewportUniform {
            viewport_size: [1.0, 1.0],
            time_seconds: 0.0,
            _padding: 0.0,
        }),
        usage: BufferUsages::UNIFORM | BufferUsages::COPY_DST,
    });
    let sampler = render_device.create_sampler(&SamplerDescriptor {
        label: Some("shared nearest UI texture sampler"),
        address_mode_u: AddressMode::ClampToEdge,
        address_mode_v: AddressMode::ClampToEdge,
        address_mode_w: AddressMode::ClampToEdge,
        mag_filter: FilterMode::Nearest,
        min_filter: FilterMode::Nearest,
        mipmap_filter: FilterMode::Nearest,
        ..default()
    });
    commands.insert_resource(UiGpu {
        device: render_device.wgpu_device().clone(),
        device_observation: DeviceObservation::new(tick.this_run()),
        vertex_buffer: None,
        index_buffer: None,
        vertex_capacity: 0,
        index_capacity: 0,
        vertex_arena_id: 0,
        index_arena_id: 0,
        viewport_buffer,
        viewport_size: [1, 1],
        started: std::time::Instant::now(),
        textures: UiGpuTextures::default(),
        sampler,
        batches: Arc::from([]),
        accepted_revision: None,
        last_admitted_revision: None,
        last_admitted_publication: Weak::new(),
        index_count: 0,
        uploads: uploads::BufferUploads::default(),
        view_pipelines: std::collections::BTreeMap::new(),
    });
}

pub(crate) fn prepare_ui_resources(
    scene: Res<UiRenderScene>,
    render_device: Res<RenderDevice>,
    render_queue: Res<RenderQueue>,
    mut gpu: ResMut<UiGpu>,
    stats: Res<UiRenderStats>,
    tick: SystemChangeTick,
    coverage: Option<Res<UiHandCoverage>>,
) {
    let same_device = &gpu.device == render_device.wgpu_device();
    let device_valid =
        gpu.device_observation
            .observe(render_device.last_changed(), tick.this_run(), same_device);
    if let Some(coverage) = coverage {
        coverage.clear();
    }
    let Some(input) = scene.input.as_ref() else {
        gpu.accepted_revision = None;
        gpu.batches = Arc::from([]);
        stats.update(|s| {
            s.accepted_revision = None;
            s.draw_calls = 0;
        });
        return;
    };
    if !device_valid {
        gpu.accepted_revision = None;
        gpu.batches = Arc::from([]);
        record_render_rejection(
            &stats,
            input.revision,
            UiRenderRejectReason::InvalidTextureExtent,
        );
        return;
    }
    // Written every frame: the glint animates without a new UI revision.
    let viewport = UiViewportUniform {
        viewport_size: [input.viewport_size[0] as f32, input.viewport_size[1] as f32],
        time_seconds: gpu.started.elapsed().as_secs_f32() % 3600.0,
        _padding: 0.0,
    };
    render_queue.write_buffer(&gpu.viewport_buffer, 0, bytemuck::bytes_of(&viewport));
    if let Some(previous) = gpu.last_admitted_revision {
        let reason = if input.revision < previous {
            Some(UiRenderRejectReason::StaleRevision {
                current: previous,
                rejected: input.revision,
            })
        } else if input.revision == previous
            && !gpu.last_admitted_publication.ptr_eq(&Arc::downgrade(input))
        {
            Some(UiRenderRejectReason::RevisionConflict {
                revision: input.revision,
            })
        } else {
            None
        };
        if let Some(reason) = reason {
            gpu.accepted_revision = None;
            gpu.batches = Arc::from([]);
            record_render_rejection(&stats, input.revision, reason);
            return;
        }
    }
    if gpu.accepted_revision == Some(input.revision) {
        if !gpu.textures.resident(&input.textures)
            || (!input.vertices.is_empty() && gpu.vertex_buffer.is_none())
            || (!input.indices.is_empty() && gpu.index_buffer.is_none())
        {
            gpu.accepted_revision = None;
            gpu.batches = Arc::from([]);
            record_render_rejection(
                &stats,
                input.revision,
                UiRenderRejectReason::InvalidTextureExtent,
            );
        }
        return;
    }
    if let Err(reason) = input.validate() {
        gpu.accepted_revision = None;
        gpu.batches = Arc::from([]);
        record_render_rejection(&stats, input.revision, reason);
        return;
    }
    if let Err(reason) = gpu
        .textures
        .prepare(&input.textures, &render_device, &render_queue)
    {
        gpu.accepted_revision = None;
        gpu.batches = Arc::from([]);
        record_render_rejection(&stats, input.revision, reason);
        return;
    }

    let fresh_vertices = gpu.vertex_capacity < input.vertices.len();
    let fresh_indices = gpu.index_capacity < input.indices.len();
    if fresh_vertices {
        let capacity = arena_capacity(input.vertices.len(), MAX_UI_VERTICES);
        gpu.vertex_buffer = Some(render_device.create_buffer(&BufferDescriptor {
            label: Some("shared bounded UI vertex arena"),
            size: arena_bytes(capacity, size_of::<UiRenderVertex>()),
            usage: BufferUsages::VERTEX | BufferUsages::COPY_DST,
            mapped_at_creation: false,
        }));
        gpu.vertex_capacity = capacity;
        gpu.vertex_arena_id = gpu.vertex_arena_id.saturating_add(1);
    }
    if fresh_indices {
        let capacity = arena_capacity(input.indices.len(), MAX_UI_INDICES);
        gpu.index_buffer = Some(render_device.create_buffer(&BufferDescriptor {
            label: Some("shared bounded UI index arena"),
            size: arena_bytes(capacity, size_of::<u32>()),
            usage: BufferUsages::INDEX | BufferUsages::COPY_DST,
            mapped_at_creation: false,
        }));
        gpu.index_capacity = capacity;
        gpu.index_arena_id = gpu.index_arena_id.saturating_add(1);
    }
    let upload = gpu.uploads.plan(input, fresh_vertices, fresh_indices);
    if let Some(buffer) = gpu.vertex_buffer.as_ref()
        && !upload.vertices.is_empty()
    {
        render_queue.write_buffer(
            buffer,
            (upload.vertices.start * size_of::<UiRenderVertex>()) as u64,
            bytemuck::cast_slice(&input.vertices[upload.vertices.clone()]),
        );
    }
    if let Some(buffer) = gpu.index_buffer.as_ref()
        && !upload.indices.is_empty()
    {
        render_queue.write_buffer(
            buffer,
            (upload.indices.start * size_of::<u32>()) as u64,
            bytemuck::cast_slice(&input.indices[upload.indices.clone()]),
        );
    }
    gpu.viewport_size = input.viewport_size;

    gpu.batches = Arc::clone(&input.batches);
    gpu.index_count = input.indices.len();
    gpu.accepted_revision = Some(input.revision);
    gpu.last_admitted_revision = Some(input.revision);
    gpu.last_admitted_publication = Arc::downgrade(input);
    stats.update(|stats| {
        stats.accepted_revision = Some(input.revision);
        stats.uploaded_vertices = upload.vertices.len() as u32;
        stats.uploaded_indices = upload.indices.len() as u32;
        stats.draw_calls = input.batches.len() as u32;
        stats.vertex_arena_capacity = gpu.vertex_capacity as u32;
        stats.index_arena_capacity = gpu.index_capacity as u32;
        stats.per_node_gpu_allocations = 0;
        stats.retained_gpu_bytes =
            retained_gpu_bytes(gpu.vertex_capacity, gpu.index_capacity, gpu.textures.bytes);
        stats.rejected_revision = None;
        stats.rejected_reason = None;
    });
}

fn record_render_rejection(stats: &UiRenderStats, revision: u64, reason: UiRenderRejectReason) {
    static REJECTIONS: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let count = REJECTIONS.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
    if count.is_power_of_two() {
        bevy::log::warn!(count, revision, ?reason, "UI frame rejected");
    }
    stats.update(|stats| {
        stats.accepted_revision = None;
        stats.draw_calls = 0;
        stats.rejected_revision = Some(revision);
        stats.rejected_reason = Some(reason);
        stats.rejection_count = stats.rejection_count.saturating_add(1);
    });
}

fn arena_capacity(required: usize, limit: usize) -> usize {
    if required == 0 {
        return 0;
    }
    required
        .checked_next_power_of_two()
        .unwrap_or(limit)
        .min(limit)
}

fn arena_bytes(capacity: usize, stride: usize) -> u64 {
    u64::try_from(capacity.saturating_mul(stride).max(4)).expect("bounded UI arena byte count")
}

fn retained_gpu_bytes(vertices: usize, indices: usize, texture_bytes: usize) -> u64 {
    let bytes = vertices
        .saturating_mul(size_of::<UiRenderVertex>())
        .saturating_add(indices.saturating_mul(size_of::<u32>()))
        .saturating_add(texture_bytes)
        .saturating_add(size_of::<UiViewportUniform>());
    bytes as u64
}

struct UiPipelineSpecializer;

#[derive(Resource)]
struct UiPipeline {
    variants: Variants<RenderPipeline, UiPipelineSpecializer>,
    bind_group_layout: BindGroupLayoutDescriptor,
}

impl FromWorld for UiPipeline {
    fn from_world(_world: &mut World) -> Self {
        let bind_group_layout = ui_bind_group_layout();
        let descriptor = ui_pipeline_descriptor(bind_group_layout.clone());
        Self {
            variants: Variants::new(UiPipelineSpecializer, descriptor),
            bind_group_layout,
        }
    }
}

pub(crate) fn ui_bind_group_layout() -> BindGroupLayoutDescriptor {
    BindGroupLayoutDescriptor::new(
        "shared UI bind group layout",
        &[
            BindGroupLayoutEntry {
                binding: 0,
                // The fragment stage reads the glint clock.
                visibility: ShaderStages::VERTEX_FRAGMENT,
                ty: BindingType::Buffer {
                    ty: BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: BufferSize::new(size_of::<UiViewportUniform>() as u64),
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
        ],
    )
}

/// The premultiplied-alpha blend state shared by every UI quad except the
/// crosshair.
pub(crate) fn ui_alpha_blend_state() -> BlendState {
    let blend = BlendComponent {
        src_factor: BlendFactor::One,
        dst_factor: BlendFactor::OneMinusSrcAlpha,
        operation: BlendOperation::Add,
    };
    BlendState {
        color: blend,
        alpha: blend,
    }
}

/// The classic crosshair invert: color = src*(1-dst) + dst*(1-src), so the
/// white cross reads against any background; alpha passes the source through.
pub(crate) fn ui_invert_blend_state() -> BlendState {
    BlendState {
        color: BlendComponent {
            src_factor: BlendFactor::OneMinusDst,
            dst_factor: BlendFactor::OneMinusSrc,
            operation: BlendOperation::Add,
        },
        alpha: BlendComponent {
            src_factor: BlendFactor::One,
            dst_factor: BlendFactor::Zero,
            operation: BlendOperation::Add,
        },
    }
}

pub(crate) fn ui_pipeline_descriptor(
    bind_group_layout: BindGroupLayoutDescriptor,
) -> RenderPipelineDescriptor {
    RenderPipelineDescriptor {
        label: Some("shared retained UI overlay pipeline".into()),
        layout: vec![bind_group_layout],
        vertex: VertexState {
            shader: UI_SHADER_HANDLE,
            entry_point: Some("ui_vertex".into()),
            buffers: vec![VertexBufferLayout {
                array_stride: size_of::<UiRenderVertex>() as u64,
                step_mode: VertexStepMode::Vertex,
                attributes: vec![
                    VertexAttribute {
                        format: VertexFormat::Float32x2,
                        offset: 0,
                        shader_location: 0,
                    },
                    VertexAttribute {
                        format: VertexFormat::Uint16x2,
                        offset: 8,
                        shader_location: 1,
                    },
                    VertexAttribute {
                        format: VertexFormat::Unorm8x4,
                        offset: 12,
                        shader_location: 2,
                    },
                    VertexAttribute {
                        format: VertexFormat::Uint32,
                        offset: 16,
                        shader_location: 3,
                    },
                ],
            }],
            ..default()
        },
        fragment: Some(FragmentState {
            shader: UI_SHADER_HANDLE,
            entry_point: Some("ui_fragment".into()),
            targets: vec![Some(ColorTargetState {
                format: TextureFormat::bevy_default(),
                blend: Some(ui_alpha_blend_state()),
                write_mask: ColorWrites::ALL,
            })],
            ..default()
        }),
        depth_stencil: None,
        ..default()
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, SpecializerKey)]
struct UiPipelineKey {
    msaa: Msaa,
    hdr: bool,
    invert_blend: bool,
}

impl Specializer<RenderPipeline> for UiPipelineSpecializer {
    type Key = UiPipelineKey;

    fn specialize(
        &self,
        key: Self::Key,
        descriptor: &mut RenderPipelineDescriptor,
    ) -> Result<Canonical<Self::Key>, BevyError> {
        descriptor.multisample.count = key.msaa.samples();
        let target = descriptor.fragment.as_mut().unwrap().targets[0]
            .as_mut()
            .unwrap();
        target.format = if key.hdr {
            ViewTarget::TEXTURE_FORMAT_HDR
        } else {
            TextureFormat::bevy_default()
        };
        target.blend = Some(if key.invert_blend {
            ui_invert_blend_state()
        } else {
            ui_alpha_blend_state()
        });
        Ok(key)
    }
}

fn prepare_ui_bind_group(
    render_device: Res<RenderDevice>,
    pipeline_cache: Res<PipelineCache>,
    pipeline: Res<UiPipeline>,
    mut gpu: ResMut<UiGpu>,
) {
    if gpu.accepted_revision.is_none() || &gpu.device != render_device.wgpu_device() {
        return;
    }
    let viewport = gpu.viewport_buffer.clone();
    let sampler = gpu.sampler.clone();
    for bucket in &mut gpu.textures.buckets {
        if bucket.bind_group.is_some() {
            continue;
        }
        bucket.bind_group = Some(render_device.create_bind_group(
            "shared retained UI bind group",
            &pipeline_cache.get_bind_group_layout(&pipeline.bind_group_layout),
            &[
                BindGroupEntry {
                    binding: 0,
                    resource: viewport.as_entire_binding(),
                },
                BindGroupEntry {
                    binding: 1,
                    resource: BindingResource::TextureView(&bucket.view),
                },
                BindGroupEntry {
                    binding: 2,
                    resource: BindingResource::Sampler(&sampler),
                },
            ],
        ));
    }
}

/// Resolve the entire ordered frame before emitting any batch command.
fn resolved_batches<'a>(
    accepted_revision: Option<u64>,
    batches: &'a [UiRenderBatch],
    locations: &'a [crate::UiTextureLocation],
    buckets: &[crate::UiTextureBucket],
) -> Option<impl Iterator<Item = (usize, &'a UiRenderBatch, crate::UiTextureLocation)>> {
    if accepted_revision.is_none()
        || batches.iter().any(|batch| {
            locations
                .get(batch.texture_page as usize)
                .is_none_or(|location| {
                    buckets
                        .get(location.bucket)
                        .is_none_or(|bucket| location.layer >= bucket.layers)
                })
        })
    {
        return None;
    }
    Some(
        batches
            .iter()
            .enumerate()
            .map(move |(index, batch)| (index, batch, locations[batch.texture_page as usize])),
    )
}

#[cfg(test)]
mod ordered_command_tests {
    use super::*;

    pub(super) fn binding_world() -> World {
        use bevy::ecs::system::RunSystemOnce;
        use bevy::render::renderer::{RenderAdapter, WgpuWrapper};
        use std::{
            future::Future,
            pin::pin,
            task::{Context, Poll, Waker},
        };
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
            backends: wgpu::Backends::NOOP,
            backend_options: wgpu::BackendOptions {
                noop: wgpu::NoopBackendOptions { enable: true },
                ..Default::default()
            },
            ..Default::default()
        });
        let mut context = Context::from_waker(Waker::noop());
        let Poll::Ready(Ok(adapter)) =
            pin!(instance.request_adapter(&wgpu::RequestAdapterOptions::default()))
                .poll(&mut context)
        else {
            panic!("noop adapter must be immediate");
        };
        let Poll::Ready(Ok((device, queue))) =
            pin!(adapter.request_device(&wgpu::DeviceDescriptor::default())).poll(&mut context)
        else {
            panic!("noop device must be immediate");
        };
        let device = RenderDevice::from(device);
        let adapter = RenderAdapter(Arc::new(WgpuWrapper::new(adapter)));
        let mut world = World::new();
        world.insert_resource(PipelineCache::new(device.clone(), adapter, true));
        world.insert_resource(device);
        world.insert_resource(RenderQueue(Arc::new(WgpuWrapper::new(queue))));
        world.init_resource::<UiPipeline>();
        world.init_resource::<UiRenderStats>();
        world.init_resource::<UiRenderScene>();
        world.run_system_once(init_ui_gpu).unwrap();
        world
    }

    #[test]
    fn actual_rejected_and_empty_preparation_cannot_bind_withheld_texture_resources() {
        use bevy::ecs::system::RunSystemOnce;
        let mut world = binding_world();
        let input = UiRenderInput {
            revision: 1,
            viewport_size: [64, 64],
            safe_area: [0; 4],
            vertices: Arc::from([]),
            indices: Arc::from([]),
            batches: Arc::from([]),
            textures: Arc::new(
                crate::UiTextureCatalog::new(
                    vec![crate::UiTexturePage::owned([1, 1], Arc::from([255; 4])).unwrap()],
                    1,
                )
                .unwrap(),
            ),
        };
        let mut scene = UiRenderScene::default();
        scene
            .publish(input.clone(), world.resource::<UiRenderStats>())
            .unwrap();
        world.insert_resource(scene.clone());
        world.run_system_once(prepare_ui_resources).unwrap();
        assert_eq!(world.resource::<UiGpu>().textures.buckets.len(), 1);
        let mut invalid = input.clone();
        invalid.indices = Arc::from([u32::MAX]);
        scene.input = Some(Arc::new(invalid));
        world.insert_resource(scene.clone());
        world.run_system_once(prepare_ui_resources).unwrap();
        world.run_system_once(prepare_ui_bind_group).unwrap();
        assert!(
            world.resource::<UiGpu>().textures.buckets[0]
                .bind_group
                .is_none()
        );
        assert!(world.resource::<UiGpu>().accepted_revision.is_none());
        scene.input = None;
        world.insert_resource(scene);
        world.run_system_once(prepare_ui_resources).unwrap();
        world.run_system_once(prepare_ui_bind_group).unwrap();
        assert!(
            world.resource::<UiGpu>().textures.buckets[0]
                .bind_group
                .is_none()
        );
        // Actual initialization drops withheld resident resources and resets the
        // publication lifetime. Re-admit through the real publication path.
        world.run_system_once(init_ui_gpu).unwrap();
        let mut recovered = UiRenderScene::default();
        recovered
            .publish(input, world.resource::<UiRenderStats>())
            .unwrap();
        world.insert_resource(recovered);
        world.run_system_once(prepare_ui_resources).unwrap();
        world.run_system_once(prepare_ui_bind_group).unwrap();
        assert_eq!(world.resource::<UiGpu>().accepted_revision, Some(1));
        assert!(
            world.resource::<UiGpu>().textures.buckets[0]
                .bind_group
                .is_some()
        );
    }

    #[test]
    fn actual_preparation_uploads_changed_spans_and_refills_reallocated_arenas() {
        use bevy::ecs::system::RunSystemOnce;
        let mut world = binding_world();
        let vertex = UiRenderVertex {
            position: [0.0; 2],
            uv: [0; 2],
            color: [255; 4],
            style_flags: 0,
        };
        let mut input = UiRenderInput {
            revision: 1,
            viewport_size: [64; 2],
            safe_area: [0; 4],
            vertices: Arc::from([vertex; 4]),
            indices: Arc::from([0, 1, 2, 0, 2, 3]),
            batches: Arc::from([UiRenderBatch::new(0, UiScissor::new(0, 0, 64, 64), 0, 6, 0)]),
            textures: Arc::new(
                crate::UiTextureCatalog::new(
                    vec![crate::UiTexturePage::owned([1, 1], Arc::from([255; 4])).unwrap()],
                    1,
                )
                .unwrap(),
            ),
        };
        let mut scene = UiRenderScene::default();
        for revision in 1..=4 {
            input.revision = revision;
            match revision {
                2 => Arc::make_mut(&mut input.vertices)[1].position[0] = 1.0,
                3 => input.viewport_size = [128; 2],
                4 => input.vertices = vec![vertex; 4_000].into(),
                _ => {}
            }
            scene
                .publish(input.clone(), world.resource::<UiRenderStats>())
                .unwrap();
            world.insert_resource(scene.clone());
            world.run_system_once(prepare_ui_resources).unwrap();
            let stats = world.resource::<UiRenderStats>().snapshot();
            assert_eq!(stats.accepted_revision, Some(revision));
            assert_eq!(
                stats.uploaded_vertices,
                [4, 1, 0, 4_000][revision as usize - 1]
            );
            assert_eq!(stats.uploaded_indices, if revision == 1 { 6 } else { 0 });
            assert_eq!(world.resource::<UiGpu>().viewport_size, input.viewport_size);
        }
    }

    #[test]
    fn resolved_commands_keep_bucket_layer_blend_scissor_and_index_order() {
        let plan =
            crate::UiTexturePlan::new(&[[1024, 1024], [2048, 2048], [256, 256], [2048, 2048]])
                .unwrap();
        let batches = [2, 0, 3, 1, 2]
            .into_iter()
            .enumerate()
            .map(|(index, page)| {
                UiRenderBatch::new(
                    page,
                    UiScissor::new(index as u32, 2, 30, 40),
                    index as u32 * 6,
                    6,
                    if index == 2 { UI_BLEND_INVERT } else { 0 },
                )
            })
            .collect::<Vec<_>>();
        let trace = resolved_batches(Some(7), &batches, plan.locations(), plan.buckets())
            .unwrap()
            .map(|(index, batch, location)| {
                (
                    index,
                    location.bucket,
                    location.layer,
                    batch.blend_mode,
                    batch.scissor,
                    batch.first_index..batch.first_index + batch.index_count,
                )
            })
            .collect::<Vec<_>>();
        assert_eq!(
            trace,
            vec![
                (0, 2, 0, 0, UiScissor::new(0, 2, 30, 40), 0..6),
                (1, 0, 0, 0, UiScissor::new(1, 2, 30, 40), 6..12),
                (
                    2,
                    1,
                    1,
                    UI_BLEND_INVERT,
                    UiScissor::new(2, 2, 30, 40),
                    12..18
                ),
                (3, 1, 0, 0, UiScissor::new(3, 2, 30, 40), 18..24),
                (4, 2, 0, 0, UiScissor::new(4, 2, 30, 40), 24..30),
            ]
        );
        assert!(
            resolved_batches(None, &batches, plan.locations(), plan.buckets()).is_none(),
            "rejected frame emits no commands"
        );
        let mut malformed = batches.clone();
        malformed.last_mut().unwrap().texture_page = 99;
        assert!(
            resolved_batches(Some(7), &malformed, plan.locations(), plan.buckets()).is_none(),
            "invalid late mapping must not emit a partial prefix"
        );
        let mut locations = plan.locations().to_vec();
        locations.push(crate::UiTextureLocation {
            bucket: 2,
            layer: 1,
        });
        malformed = batches;
        malformed.last_mut().unwrap().texture_page = 4;
        assert!(
            resolved_batches(Some(7), &malformed, &locations, plan.buckets()).is_none(),
            "late layer outside the actual one-layer bucket emits no prefix"
        );
    }
}

#[cfg(test)]
#[allow(dead_code)]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UiPreparedFrame {
    pub revision: u64,
    pub pipeline_id: u64,
    pub bind_group_family_id: u64,
    pub vertex_arena_id: u64,
    pub index_arena_id: u64,
    pub per_node_gpu_allocations: u32,
    draw_order: Arc<[usize]>,
    scissors: Arc<[UiScissor]>,
}

#[cfg(test)]
#[allow(dead_code)]
impl UiPreparedFrame {
    #[must_use]
    pub fn draw_order(&self) -> &[usize] {
        &self.draw_order
    }

    #[must_use]
    pub fn scissors(&self) -> &[UiScissor] {
        &self.scissors
    }
}

#[cfg(test)]
#[allow(dead_code)]
pub struct UiRenderHarness {
    scene: UiRenderScene,
    stats: UiRenderStats,
    vertex_capacity: usize,
    index_capacity: usize,
    vertex_arena_id: u64,
    index_arena_id: u64,
    prepared: Option<UiPreparedFrame>,
}

#[cfg(test)]
#[allow(dead_code)]
impl UiRenderHarness {
    #[must_use]
    pub fn new() -> Self {
        Self {
            scene: UiRenderScene::default(),
            stats: UiRenderStats::default(),
            vertex_capacity: 0,
            index_capacity: 0,
            vertex_arena_id: 0,
            index_arena_id: 0,
            prepared: None,
        }
    }

    pub fn publish(&mut self, input: UiRenderInput) -> Result<(), UiRenderReject> {
        self.scene.publish(input, &self.stats)
    }

    pub fn prepare(&mut self) -> Result<UiPreparedFrame, UiRenderReject> {
        let Some(input) = self.scene.input.as_ref() else {
            return Err(UiRenderReject {
                revision: self.scene.revision,
                reason: UiRenderRejectReason::NoPublishedScene,
            });
        };
        if let Some(prepared) = &self.prepared
            && prepared.revision == input.revision
        {
            return Ok(prepared.clone());
        }
        if self.vertex_capacity < input.vertices.len() {
            self.vertex_capacity = arena_capacity(input.vertices.len(), MAX_UI_VERTICES);
            self.vertex_arena_id = self.vertex_arena_id.saturating_add(1);
        }
        if self.index_capacity < input.indices.len() {
            self.index_capacity = arena_capacity(input.indices.len(), MAX_UI_INDICES);
            self.index_arena_id = self.index_arena_id.saturating_add(1);
        }
        self.stats.update(|stats| {
            stats.accepted_revision = Some(input.revision);
            stats.uploaded_vertices = input.vertices.len() as u32;
            stats.uploaded_indices = input.indices.len() as u32;
            stats.draw_calls = input.batches.len() as u32;
            stats.vertex_arena_capacity = self.vertex_capacity as u32;
            stats.index_arena_capacity = self.index_capacity as u32;
            stats.per_node_gpu_allocations = 0;
            stats.retained_gpu_bytes = retained_gpu_bytes(
                self.vertex_capacity,
                self.index_capacity,
                input.textures.plan().bytes(),
            );
        });
        let prepared = UiPreparedFrame {
            revision: input.revision,
            pipeline_id: 1,
            bind_group_family_id: 1,
            vertex_arena_id: self.vertex_arena_id,
            index_arena_id: self.index_arena_id,
            per_node_gpu_allocations: 0,
            draw_order: (0..input.batches.len()).collect::<Vec<_>>().into(),
            scissors: input
                .batches
                .iter()
                .map(|batch| batch.scissor)
                .collect::<Vec<_>>()
                .into(),
        };
        self.prepared = Some(prepared.clone());
        Ok(prepared)
    }

    #[must_use]
    pub const fn scene(&self) -> &UiRenderScene {
        &self.scene
    }

    #[must_use]
    pub fn stats(&self) -> crate::ui::UiRenderStatsSnapshot {
        self.stats.snapshot()
    }
}

#[cfg(test)]
impl Default for UiRenderHarness {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
#[path = "ui_render/retained_tests.rs"]
mod retained_tests;
