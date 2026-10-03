//! Draws the camera overlay stack as one full-screen triangle after all other transparent geometry.
use crate::screen_overlay::{
    MAX_SCREEN_OVERLAY_LAYERS, SCREEN_OVERLAY_TEXTURE_SIDE, ScreenOverlayScene,
};
use bevy::{
    asset::{load_internal_asset, uuid_handle},
    core_pipeline::core_3d::{CORE_3D_DEPTH_FORMAT, Transparent3d},
    ecs::{
        query::ROQueryItem,
        system::{SystemParamItem, lifetimeless::SRes},
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
            BufferBindingType, BufferInitDescriptor, BufferSize, BufferUsages, Canonical,
            ColorTargetState, ColorWrites, CompareFunction, DepthStencilState, Extent3d,
            FilterMode, FragmentState, PipelineCache, RenderPipeline, RenderPipelineDescriptor,
            Sampler, SamplerBindingType, SamplerDescriptor, ShaderStages, Specializer,
            SpecializerKey, Texture, TextureDataOrder, TextureDescriptor, TextureDimension,
            TextureFormat, TextureSampleType, TextureUsages, TextureView, TextureViewDescriptor,
            TextureViewDimension, Variants, VertexState,
        },
        renderer::{RenderDevice, RenderQueue},
        sync_world::MainEntity,
        view::{ExtractedView, ViewTarget},
    },
};

const OVERLAY_SHADER_HANDLE: Handle<Shader> = uuid_handle!("2f6d4c1a-8b73-4e0c-a5d9-61c7b3e8f204");
const UNIFORM_BYTES: usize = std::mem::size_of::<OverlayUniform>();

/// Mirrors `Overlays` in `screen_overlay.wgsl`: each layer is `[r, g, b, alpha, kind, 0, 0, 0]`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
struct OverlayUniform {
    header: [f32; 4],
    layers: [[f32; 8]; MAX_SCREEN_OVERLAY_LAYERS],
}

#[derive(Clone, Copy, Debug, Default)]
pub struct ScreenOverlayRenderPlugin;

impl Plugin for ScreenOverlayRenderPlugin {
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
    app.init_resource::<ScreenOverlayScene>();
    let Some(render_app) = app.get_sub_app(RenderApp) else {
        return;
    };
    if render_app.world().contains_resource::<Installed>() {
        return;
    }
    app.add_plugins(ExtractResourcePlugin::<ScreenOverlayScene>::default());
    load_internal_asset!(
        app,
        OVERLAY_SHADER_HANDLE,
        "screen_overlay.wgsl",
        Shader::from_wgsl
    );
    app.sub_app_mut(RenderApp)
        .insert_resource(Installed)
        .init_resource::<OverlayPipeline>()
        .add_render_command::<Transparent3d, DrawOverlayCommands>()
        .add_systems(RenderStartup, init_gpu)
        .add_systems(
            Render,
            (
                prepare_overlay.in_set(RenderSystems::PrepareResources),
                prepare_bind_group.in_set(RenderSystems::PrepareBindGroups),
                queue_overlay
                    .run_if(crate::panorama::world_passes_enabled)
                    .in_set(RenderSystems::Queue),
            ),
        );
}

#[derive(Resource)]
struct OverlayGpu {
    uniform: Buffer,
    sampler: Sampler,
    _texture: Texture,
    texture_view: TextureView,
    textures_revision: Option<u64>,
    layer_count: u32,
    bind_group: Option<BindGroup>,
}

fn texture_array(
    device: &RenderDevice,
    queue: &RenderQueue,
    side: u32,
    pixels: &[u8],
) -> (Texture, TextureView) {
    let texture = device.create_texture_with_data(
        queue,
        &TextureDescriptor {
            label: Some("screen overlay textures"),
            size: Extent3d {
                width: side,
                height: side,
                depth_or_array_layers: 2,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: TextureDimension::D2,
            format: TextureFormat::Rgba8UnormSrgb,
            usage: TextureUsages::TEXTURE_BINDING | TextureUsages::COPY_DST,
            view_formats: &[],
        },
        TextureDataOrder::LayerMajor,
        pixels,
    );
    let view = texture.create_view(&TextureViewDescriptor {
        label: Some("screen overlay texture layers"),
        dimension: Some(TextureViewDimension::D2Array),
        ..default()
    });
    (texture, view)
}

fn init_gpu(mut commands: Commands, device: Res<RenderDevice>, queue: Res<RenderQueue>) {
    let (texture, texture_view) = texture_array(&device, &queue, 1, &[255; 8]);
    commands.insert_resource(OverlayGpu {
        uniform: device.create_buffer_with_data(&BufferInitDescriptor {
            label: Some("screen overlay layers"),
            contents: bytemuck::bytes_of(&OverlayUniform::default()),
            usage: BufferUsages::UNIFORM | BufferUsages::COPY_DST,
        }),
        sampler: device.create_sampler(&SamplerDescriptor {
            label: Some("screen overlay sampler"),
            address_mode_u: AddressMode::ClampToEdge,
            address_mode_v: AddressMode::ClampToEdge,
            address_mode_w: AddressMode::ClampToEdge,
            mag_filter: FilterMode::Linear,
            min_filter: FilterMode::Linear,
            mipmap_filter: FilterMode::Nearest,
            ..default()
        }),
        _texture: texture,
        texture_view,
        textures_revision: None,
        layer_count: 0,
        bind_group: None,
    });
}

fn prepare_overlay(
    scene: Res<ScreenOverlayScene>,
    device: Res<RenderDevice>,
    queue: Res<RenderQueue>,
    mut gpu: ResMut<OverlayGpu>,
) {
    if gpu.textures_revision != Some(scene.textures_revision) {
        let (texture, view) = match &scene.textures {
            Some(textures) => texture_array(
                &device,
                &queue,
                SCREEN_OVERLAY_TEXTURE_SIDE,
                &textures.layer_major(),
            ),
            None => texture_array(&device, &queue, 1, &[255; 8]),
        };
        gpu._texture = texture;
        gpu.texture_view = view;
        gpu.textures_revision = Some(scene.textures_revision);
        gpu.bind_group = None;
    }
    let count = scene.layers.len().min(MAX_SCREEN_OVERLAY_LAYERS);
    gpu.layer_count = count as u32;
    if count == 0 {
        return;
    }
    let mut uniform = OverlayUniform {
        header: [
            count as f32,
            scene.clock_seconds,
            if scene.textures.is_some() { 1.0 } else { 0.0 },
            0.0,
        ],
        layers: [[0.0; 8]; MAX_SCREEN_OVERLAY_LAYERS],
    };
    for (slot, layer) in uniform.layers.iter_mut().zip(&scene.layers) {
        *slot = [
            layer.rgb[0],
            layer.rgb[1],
            layer.rgb[2],
            layer.alpha,
            layer.kind as u32 as f32,
            0.0,
            0.0,
            0.0,
        ];
    }
    queue.write_buffer(&gpu.uniform, 0, bytemuck::bytes_of(&uniform));
}

struct OverlayPipelineSpecializer;

#[derive(Resource)]
struct OverlayPipeline {
    variants: Variants<RenderPipeline, OverlayPipelineSpecializer>,
    bind_group_layout: BindGroupLayoutDescriptor,
}

impl FromWorld for OverlayPipeline {
    fn from_world(_world: &mut World) -> Self {
        let bind_group_layout = BindGroupLayoutDescriptor::new(
            "screen overlay bind group layout",
            &[
                BindGroupLayoutEntry {
                    binding: 0,
                    visibility: ShaderStages::FRAGMENT,
                    ty: BindingType::Buffer {
                        ty: BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: BufferSize::new(UNIFORM_BYTES as u64),
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
        );
        let descriptor = RenderPipelineDescriptor {
            label: Some("screen overlay pipeline".into()),
            layout: vec![bind_group_layout.clone()],
            vertex: VertexState {
                shader: OVERLAY_SHADER_HANDLE,
                entry_point: Some("overlay_vertex".into()),
                buffers: Vec::new(),
                ..default()
            },
            fragment: Some(FragmentState {
                shader: OVERLAY_SHADER_HANDLE,
                entry_point: Some("overlay_fragment".into()),
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
            variants: Variants::new(OverlayPipelineSpecializer, descriptor),
            bind_group_layout,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, SpecializerKey)]
struct OverlayPipelineKey {
    msaa: Msaa,
    hdr: bool,
}

impl Specializer<RenderPipeline> for OverlayPipelineSpecializer {
    type Key = OverlayPipelineKey;

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
    pipeline_cache: Res<PipelineCache>,
    pipeline: Res<OverlayPipeline>,
    mut gpu: ResMut<OverlayGpu>,
) {
    if gpu.bind_group.is_some() {
        return;
    }
    gpu.bind_group = Some(device.create_bind_group(
        "screen overlay bind group",
        &pipeline_cache.get_bind_group_layout(&pipeline.bind_group_layout),
        &[
            BindGroupEntry {
                binding: 0,
                resource: gpu.uniform.as_entire_binding(),
            },
            BindGroupEntry {
                binding: 1,
                resource: BindingResource::TextureView(&gpu.texture_view),
            },
            BindGroupEntry {
                binding: 2,
                resource: BindingResource::Sampler(&gpu.sampler),
            },
        ],
    ));
}

fn queue_overlay(
    pipeline_cache: Res<PipelineCache>,
    mut pipeline: ResMut<OverlayPipeline>,
    scene: Res<ScreenOverlayScene>,
    mut phases: ResMut<ViewSortedRenderPhases<Transparent3d>>,
    draw_functions: Res<DrawFunctions<Transparent3d>>,
    views: Query<(Entity, &MainEntity, &ExtractedView, &Msaa)>,
) {
    if scene.layers.is_empty() {
        return;
    }
    let draw_function = draw_functions.read().id::<DrawOverlayCommands>();
    for (view_entity, main_entity, view, msaa) in &views {
        let Some(phase) = phases.get_mut(&view.retained_view_entity) else {
            continue;
        };
        let Ok(pipeline_id) = pipeline.variants.specialize(
            &pipeline_cache,
            OverlayPipelineKey {
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
            // Sorts after every real transparent item.
            distance: f32::MAX,
            batch_range: 0..1,
            extra_index: PhaseItemExtraIndex::None,
            indexed: false,
        });
    }
}

type DrawOverlayCommands = (SetItemPipeline, SetOverlayBindGroup, DrawOverlay);

struct SetOverlayBindGroup;

impl<P: PhaseItem> RenderCommand<P> for SetOverlayBindGroup {
    type Param = SRes<OverlayGpu>;
    type ViewQuery = ();
    type ItemQuery = ();

    fn render<'w>(
        _item: &P,
        _view: ROQueryItem<'w, '_, Self::ViewQuery>,
        _item_query: Option<ROQueryItem<'w, '_, Self::ItemQuery>>,
        gpu: SystemParamItem<'w, '_, Self::Param>,
        pass: &mut TrackedRenderPass<'w>,
    ) -> RenderCommandResult {
        let Some(bind_group) = &gpu.into_inner().bind_group else {
            return RenderCommandResult::Skip;
        };
        pass.set_bind_group(0, bind_group, &[]);
        RenderCommandResult::Success
    }
}

struct DrawOverlay;

impl<P: PhaseItem> RenderCommand<P> for DrawOverlay {
    type Param = SRes<OverlayGpu>;
    type ViewQuery = ();
    type ItemQuery = ();

    fn render<'w>(
        _item: &P,
        _view: ROQueryItem<'w, '_, Self::ViewQuery>,
        _item_query: Option<ROQueryItem<'w, '_, Self::ItemQuery>>,
        gpu: SystemParamItem<'w, '_, Self::Param>,
        pass: &mut TrackedRenderPass<'w>,
    ) -> RenderCommandResult {
        if gpu.into_inner().layer_count == 0 {
            return RenderCommandResult::Skip;
        }
        pass.draw(0..3, 0..1);
        RenderCommandResult::Success
    }
}

#[cfg(test)]
mod tests {
    use super::UNIFORM_BYTES;

    #[test]
    fn uniform_matches_the_wgsl_layout() {
        assert_eq!(UNIFORM_BYTES, 16 + 8 * 32);
    }
}

#[cfg(test)]
mod review_tests {
    use super::*;
    use crate::queue_review_support as fixture;
    use bevy::ecs::system::RunSystemOnce;
    #[test]
    fn review_render_overlay_queue_uses_current_layers() {
        let (mut app, view) = fixture::app();
        app.init_resource::<ScreenOverlayScene>()
            .init_resource::<OverlayPipeline>()
            .add_render_command::<Transparent3d, DrawOverlayCommands>();
        app.world_mut().run_system_once(init_gpu).unwrap();
        app.world_mut()
            .resource_mut::<ScreenOverlayScene>()
            .set_layers(
                [crate::ScreenOverlayLayer {
                    kind: crate::ScreenOverlayKind::Flat,
                    rgb: [1.0; 3],
                    alpha: 1.0,
                }],
                0.0,
            );
        app.world_mut().run_system_once(queue_overlay).unwrap();
        assert_eq!(fixture::items(&app, view).len(), 1);
        fixture::clear(&mut app, view);
        app.world_mut().resource_mut::<OverlayGpu>().layer_count = 1;
        app.world_mut()
            .resource_mut::<ScreenOverlayScene>()
            .set_layers([], 0.0);
        app.world_mut().run_system_once(queue_overlay).unwrap();
        assert!(fixture::items(&app, view).is_empty());
    }
}
