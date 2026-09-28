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
        render_phase::{
            AddRenderCommand, DrawFunctions, PhaseItem, PhaseItemExtraIndex, RenderCommand,
            RenderCommandResult, SetItemPipeline, TrackedRenderPass, ViewSortedRenderPhases,
        },
        render_resource::{
            BindGroup, BindGroupEntry, BindGroupLayoutDescriptor, BindGroupLayoutEntry,
            BindingType, BlendState, Buffer, BufferBindingType, BufferId, BufferInitDescriptor,
            BufferSize, BufferUsages, Canonical, ColorTargetState, ColorWrites, CompareFunction,
            DepthStencilState, FragmentState, PipelineCache, RenderPipeline,
            RenderPipelineDescriptor, ShaderStages, ShaderType, Specializer, SpecializerKey,
            TextureFormat, Variants, VertexState,
        },
        renderer::{RenderDevice, RenderQueue},
        sync_world::MainEntity,
        view::{ExtractedView, ViewTarget, ViewUniform, ViewUniformOffset, ViewUniforms},
    },
};

use crate::{
    AtmosphereFrame,
    atmosphere_render::AtmosphereGpu,
    weather::{
        MAX_PRECIPITATION_COLUMNS, PRECIPITATION_ABOVE_CAMERA, PrecipitationColumn,
        PrecipitationScene, precipitation_wind,
    },
};

const WEATHER_SHADER_HANDLE: Handle<Shader> = uuid_handle!("5b0f2f6e-6c1d-4a58-9f0a-3f1d7a9e2c11");
const COLUMN_BYTES: usize = std::mem::size_of::<PrecipitationColumn>();
const PARAMS_BYTES: usize = std::mem::size_of::<WeatherParamsGpu>();

/// Uniform read by `weather.wgsl`: `clock` is time, opacity and wind xz; `extent.x` the sheet height.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
struct WeatherParamsGpu {
    clock: [f32; 4],
    extent: [f32; 4],
}

pub(crate) fn install_weather_render(app: &mut App) {
    load_internal_asset!(
        app,
        WEATHER_SHADER_HANDLE,
        "weather.wgsl",
        Shader::from_wgsl
    );
    app.sub_app_mut(RenderApp)
        .init_resource::<WeatherPipeline>()
        .add_render_command::<Transparent3d, DrawWeatherCommands>()
        .add_systems(RenderStartup, init_weather_gpu)
        .add_systems(
            Render,
            (
                prepare_weather_records.in_set(RenderSystems::PrepareResources),
                prepare_weather_bind_group.in_set(RenderSystems::PrepareBindGroups),
                queue_weather.in_set(RenderSystems::Queue),
            ),
        );
}

#[derive(Resource)]
pub(crate) struct WeatherGpu {
    record_buffer: Buffer,
    params_buffer: Buffer,
    pub(crate) column_count: u32,
    bind_group: Option<BindGroup>,
    view_buffer_id: Option<BufferId>,
    atmosphere_buffer_id: Option<BufferId>,
}

fn init_weather_gpu(mut commands: Commands, render_device: Res<RenderDevice>) {
    let record_buffer = render_device.create_buffer_with_data(&BufferInitDescriptor {
        label: Some("precipitation column records"),
        contents: &vec![0_u8; MAX_PRECIPITATION_COLUMNS * COLUMN_BYTES],
        usage: BufferUsages::STORAGE | BufferUsages::COPY_DST,
    });
    let params_buffer = render_device.create_buffer_with_data(&BufferInitDescriptor {
        label: Some("precipitation parameters"),
        contents: bytemuck::bytes_of(&WeatherParamsGpu::default()),
        usage: BufferUsages::UNIFORM | BufferUsages::COPY_DST,
    });
    commands.insert_resource(WeatherGpu {
        record_buffer,
        params_buffer,
        column_count: 0,
        bind_group: None,
        view_buffer_id: None,
        atmosphere_buffer_id: None,
    });
}

pub(crate) fn prepare_weather_records(
    scene: Res<PrecipitationScene>,
    render_queue: Res<RenderQueue>,
    mut gpu: ResMut<WeatherGpu>,
) {
    let count = scene.columns.len().min(MAX_PRECIPITATION_COLUMNS);
    gpu.column_count = u32::try_from(count).expect("bounded precipitation column count");
    if count == 0 {
        return;
    }
    render_queue.write_buffer(
        &gpu.record_buffer,
        0,
        bytemuck::cast_slice::<PrecipitationColumn, u8>(&scene.columns[..count]),
    );
    let wind = precipitation_wind(scene.clock);
    let params = WeatherParamsGpu {
        clock: [scene.clock, scene.level, wind[0], wind[1]],
        extent: [PRECIPITATION_ABOVE_CAMERA, 0.0, 0.0, 0.0],
    };
    render_queue.write_buffer(&gpu.params_buffer, 0, bytemuck::bytes_of(&params));
}

struct WeatherPipelineSpecializer;

#[derive(Resource)]
struct WeatherPipeline {
    variants: Variants<RenderPipeline, WeatherPipelineSpecializer>,
    bind_group_layout: BindGroupLayoutDescriptor,
}

impl FromWorld for WeatherPipeline {
    fn from_world(_world: &mut World) -> Self {
        let uniform = |binding: u32, size: BufferSize, dynamic: bool| BindGroupLayoutEntry {
            binding,
            visibility: ShaderStages::VERTEX | ShaderStages::FRAGMENT,
            ty: BindingType::Buffer {
                ty: BufferBindingType::Uniform,
                has_dynamic_offset: dynamic,
                min_binding_size: Some(size),
            },
            count: None,
        };
        let bind_group_layout = BindGroupLayoutDescriptor::new(
            "precipitation bind group layout",
            &[
                uniform(0, ViewUniform::min_size(), true),
                uniform(1, AtmosphereFrame::min_size(), false),
                BindGroupLayoutEntry {
                    binding: 2,
                    visibility: ShaderStages::VERTEX,
                    ty: BindingType::Buffer {
                        ty: BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: BufferSize::new(COLUMN_BYTES as u64),
                    },
                    count: None,
                },
                uniform(
                    3,
                    BufferSize::new(PARAMS_BYTES as u64).expect("non-zero params size"),
                    false,
                ),
            ],
        );
        let descriptor = RenderPipelineDescriptor {
            label: Some("precipitation pipeline".into()),
            layout: vec![bind_group_layout.clone()],
            vertex: VertexState {
                shader: WEATHER_SHADER_HANDLE,
                entry_point: Some("weather_vertex".into()),
                buffers: Vec::new(),
                ..default()
            },
            fragment: Some(FragmentState {
                shader: WEATHER_SHADER_HANDLE,
                entry_point: Some("weather_fragment".into()),
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
                depth_compare: CompareFunction::GreaterEqual,
                stencil: default(),
                bias: default(),
            }),
            ..default()
        };
        Self {
            variants: Variants::new(WeatherPipelineSpecializer, descriptor),
            bind_group_layout,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, SpecializerKey)]
struct WeatherPipelineKey {
    msaa: Msaa,
    hdr: bool,
}

impl Specializer<RenderPipeline> for WeatherPipelineSpecializer {
    type Key = WeatherPipelineKey;

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

fn prepare_weather_bind_group(
    render_device: Res<RenderDevice>,
    pipeline_cache: Res<PipelineCache>,
    pipeline: Res<WeatherPipeline>,
    view_uniforms: Res<ViewUniforms>,
    atmosphere: Res<AtmosphereGpu>,
    mut gpu: ResMut<WeatherGpu>,
) {
    let Some(view_binding) = view_uniforms.uniforms.binding() else {
        gpu.bind_group = None;
        return;
    };
    let view_buffer = view_uniforms
        .uniforms
        .buffer()
        .expect("a dynamic view binding always owns a GPU buffer");
    if gpu.bind_group.is_some()
        && gpu.view_buffer_id == Some(view_buffer.id())
        && gpu.atmosphere_buffer_id == Some(atmosphere.buffer.id())
    {
        return;
    }
    gpu.bind_group = Some(render_device.create_bind_group(
        "precipitation bind group",
        &pipeline_cache.get_bind_group_layout(&pipeline.bind_group_layout),
        &[
            BindGroupEntry {
                binding: 0,
                resource: view_binding,
            },
            BindGroupEntry {
                binding: 1,
                resource: atmosphere.buffer.as_entire_binding(),
            },
            BindGroupEntry {
                binding: 2,
                resource: gpu.record_buffer.as_entire_binding(),
            },
            BindGroupEntry {
                binding: 3,
                resource: gpu.params_buffer.as_entire_binding(),
            },
        ],
    ));
    gpu.view_buffer_id = Some(view_buffer.id());
    gpu.atmosphere_buffer_id = Some(atmosphere.buffer.id());
}

fn queue_weather(
    pipeline_cache: Res<PipelineCache>,
    mut pipeline: ResMut<WeatherPipeline>,
    gpu: Res<WeatherGpu>,
    scene: Res<PrecipitationScene>,
    mut phases: ResMut<ViewSortedRenderPhases<Transparent3d>>,
    draw_functions: Res<DrawFunctions<Transparent3d>>,
    views: Query<(Entity, &MainEntity, &ExtractedView, &Msaa)>,
) {
    if gpu.column_count == 0 || scene.level <= 0.0 {
        return;
    }
    let draw_function = draw_functions.read().id::<DrawWeatherCommands>();
    for (view_entity, main_entity, view, msaa) in &views {
        let Some(phase) = phases.get_mut(&view.retained_view_entity) else {
            continue;
        };
        let Ok(pipeline_id) = pipeline.variants.specialize(
            &pipeline_cache,
            WeatherPipelineKey {
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
            distance: view
                .rangefinder3d()
                .distance(&Vec3::from(view.world_from_view.translation())),
            batch_range: 0..1,
            extra_index: PhaseItemExtraIndex::None,
            indexed: false,
        });
    }
}

type DrawWeatherCommands = (SetItemPipeline, SetWeatherBindGroup<0>, DrawWeather);

struct SetWeatherBindGroup<const I: usize>;

impl<P: PhaseItem, const I: usize> RenderCommand<P> for SetWeatherBindGroup<I> {
    type Param = SRes<WeatherGpu>;
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

struct DrawWeather;

impl<P: PhaseItem> RenderCommand<P> for DrawWeather {
    type Param = SRes<WeatherGpu>;
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
        pass.draw(0..gpu.column_count.saturating_mul(6), 0..1);
        RenderCommandResult::Success
    }
}

#[cfg(test)]
mod tests {
    use super::{COLUMN_BYTES, PARAMS_BYTES};

    #[test]
    fn gpu_records_match_the_wgsl_layouts() {
        assert_eq!(COLUMN_BYTES, 16);
        assert_eq!(PARAMS_BYTES, 32);
    }
}
