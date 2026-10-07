//! GPU pass timings use asynchronous timestamp readback; Metal measures actual render passes.
//! In-pass draw spans and category pass splitting are opt-in diagnostics.

mod categories;
mod health;
mod nodes;
mod opaque;
mod overdraw;
mod queries;
pub(crate) mod readback;
#[cfg(test)]
mod tests;

use crate::{RuntimeStage, RuntimeStageProfiler};
use bevy::{
    core_pipeline::core_3d::graph::{Core3d, Node3d},
    ecs::system::{SystemParamItem, lifetimeless::SRes},
    prelude::*,
    render::{
        Render, RenderApp, RenderStartup, RenderSystems,
        render_graph::{
            EmptyNode, Node, NodeRunError, RenderGraph, RenderGraphContext, RenderLabel, SlotInfo,
        },
        render_phase::{PhaseItem, RenderCommand, RenderCommandResult, TrackedRenderPass},
        renderer::{RenderAdapterInfo, RenderContext, RenderDevice, RenderQueue, render_system},
    },
};
use queries::{GpuTimestamps, Span};
use std::marker::PhantomData;

pub use readback::GpuFrameTimes;

/// Feeds `gpu_*` stages into the [`RuntimeStageProfiler`] already present in the app.
pub struct GpuTimingPlugin;

impl Plugin for GpuTimingPlugin {
    fn build(&self, app: &mut App) {
        let Some(profiler) = app.world().get_resource::<RuntimeStageProfiler>().cloned() else {
            return;
        };
        let Some(render_app) = app.get_sub_app_mut(RenderApp) else {
            return;
        };
        if overdraw::requested() {
            overdraw::install(render_app);
        }
        if categories::requested() {
            render_app.insert_resource(categories::CategoryProfiling);
        }
        render_app
            .insert_resource(profiler)
            // RenderStartup runs inside the render app after every plugin has built the graph,
            // and still reaches it after pipelined rendering moves the app to its own thread.
            .add_systems(RenderStartup, (init_gpu_timestamps, wrap_timed_nodes))
            .add_systems(
                Render,
                (
                    begin_gpu_frame.in_set(RenderSystems::PrepareResources),
                    submit_gpu_frame
                        .in_set(RenderSystems::Render)
                        .after(render_system),
                ),
            );
    }
}

fn wrap_timed_nodes(world: &mut World) {
    let metal = world
        .get_resource::<RenderAdapterInfo>()
        .is_some_and(|info| info.backend == wgpu::Backend::Metal);
    let mut replacement = categories::replacement(world).or_else(|| opaque::replacement(world));
    let Some(mut graphs) = world.get_resource_mut::<RenderGraph>() else {
        return;
    };
    let Some(graph) = graphs.get_sub_graph_mut(Core3d) else {
        return;
    };
    for (label, stage) in nodes::timed_nodes() {
        let Ok(state) = graph.get_node_state_mut(label) else {
            continue;
        };
        // Replacing the node alone preserves its slots and edges.
        let mut inner = std::mem::replace(&mut state.node, Box::new(EmptyNode));
        if label == Node3d::MainOpaquePass.intern()
            && categories::replaceable(&*inner)
            && let Some(replacement) = replacement.take()
        {
            inner = replacement;
        }
        state.node = if metal {
            inner
        } else {
            Box::new(TimedNode { inner, stage })
        };
    }
}

struct TimedNode {
    inner: Box<dyn Node>,
    stage: RuntimeStage,
}

impl Node for TimedNode {
    fn input(&self) -> Vec<SlotInfo> {
        self.inner.input()
    }

    fn output(&self) -> Vec<SlotInfo> {
        self.inner.output()
    }

    fn update(&mut self, world: &mut World) {
        self.inner.update(world);
    }

    fn run<'w>(
        &self,
        graph: &mut RenderGraphContext,
        render_context: &mut RenderContext<'w>,
        world: &'w World,
    ) -> Result<(), NodeRunError> {
        let span = world
            .get_resource::<GpuTimestamps>()
            .and_then(|timestamps| timestamps.open_pass(self.stage));
        if let Some(span) = &span {
            mark(render_context, span, span.begin);
        }
        let result = self.inner.run(graph, render_context, world);
        if let Some(span) = &span {
            mark(render_context, span, span.begin + 1);
        }
        result
    }
}

/// Times `record` as one node-level span of `stage`, for nodes that record several passes.
pub(crate) fn timed<'w, R>(
    world: &World,
    context: &mut RenderContext<'w>,
    stage: RuntimeStage,
    record: impl FnOnce(&mut RenderContext<'w>) -> R,
) -> R {
    let span = world
        .get_resource::<GpuTimestamps>()
        .filter(|timestamps| !timestamps.native_passes)
        .and_then(|timestamps| timestamps.open_pass(stage));
    if let Some(span) = &span {
        mark(context, span, span.begin);
    }
    let result = record(context);
    if let Some(span) = &span {
        mark(context, span, span.begin + 1);
    }
    result
}

/// Existing non-Metal markers remain separate from actual-pass Metal timestamps.
fn mark(context: &mut RenderContext, span: &Span<'_>, index: u32) {
    let _pass = context
        .command_encoder()
        .begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("gpu timestamp"),
            timestamp_writes: Some(wgpu::ComputePassTimestampWrites {
                query_set: span.queries,
                beginning_of_pass_write_index: None,
                end_of_pass_write_index: Some(index),
            }),
        });
}

/// Attaches timestamps to actual render work on Metal; other backends use their node spans.
pub(crate) fn pass_writes(
    world: &World,
    stage: RuntimeStage,
) -> Option<wgpu::RenderPassTimestampWrites<'_>> {
    let timestamps = world.get_resource::<GpuTimestamps>()?;
    if !timestamps.native_passes {
        return None;
    }
    let span = timestamps.open_pass(stage)?;
    Some(wgpu::RenderPassTimestampWrites {
        query_set: span.queries,
        beginning_of_pass_write_index: Some(span.begin),
        end_of_pass_write_index: Some(span.begin + 1),
    })
}

/// Times draw command `C` as stage `STAGE` (a [`RuntimeStage`] index) inside its pass.
pub(crate) struct GpuDrawSpan<const STAGE: usize, C>(PhantomData<fn() -> C>);

impl<P: PhaseItem, const STAGE: usize, C: RenderCommand<P>> RenderCommand<P>
    for GpuDrawSpan<STAGE, C>
{
    type Param = (Option<SRes<GpuTimestamps>>, C::Param);
    type ViewQuery = C::ViewQuery;
    type ItemQuery = C::ItemQuery;

    fn render<'w>(
        item: &P,
        view: bevy::ecs::query::ROQueryItem<'w, '_, Self::ViewQuery>,
        entity: Option<bevy::ecs::query::ROQueryItem<'w, '_, Self::ItemQuery>>,
        (timestamps, param): SystemParamItem<'w, '_, Self::Param>,
        pass: &mut TrackedRenderPass<'w>,
    ) -> RenderCommandResult {
        let span = timestamps
            .map(|timestamps| timestamps.into_inner())
            .and_then(|timestamps| timestamps.open_draw(RuntimeStage::ALL[STAGE]));
        if let Some(span) = &span {
            pass.wgpu_pass().write_timestamp(span.queries, span.begin);
        }
        let result = C::render(item, view, entity, param, pass);
        if let Some(span) = &span {
            pass.wgpu_pass()
                .write_timestamp(span.queries, span.begin + 1);
        }
        result
    }
}

fn init_gpu_timestamps(
    mut commands: Commands,
    device: Res<RenderDevice>,
    queue: Res<RenderQueue>,
    profiler: Res<RuntimeStageProfiler>,
    categories: Option<Res<categories::CategoryProfiling>>,
    adapter: Option<Res<RenderAdapterInfo>>,
) {
    let backend = adapter.map_or(wgpu::Backend::Noop, |adapter| adapter.backend);
    match GpuTimestamps::new(&device, &queue, profiler.enabled(), backend) {
        Some(mut timestamps) => {
            timestamps.draw_spans &= categories.is_none();
            commands.insert_resource(timestamps);
        }
        None => info!("GPU timestamps unsupported by this adapter; gpu_* stages stay empty"),
    }
}

fn begin_gpu_frame(
    timestamps: Option<ResMut<GpuTimestamps>>,
    device: Res<RenderDevice>,
    queue: Res<RenderQueue>,
    profiler: Res<RuntimeStageProfiler>,
) {
    let Some(mut timestamps) = timestamps else {
        return;
    };
    // Only fires callbacks for work the GPU has already completed.
    let _ = device.poll(wgpu::PollType::Poll);
    timestamps.resolve_ready(&device, &queue);
    timestamps.begin(|frame| profiler.record_gpu_frame(frame));
}

fn submit_gpu_frame(
    timestamps: Option<ResMut<GpuTimestamps>>,
    device: Res<RenderDevice>,
    queue: Res<RenderQueue>,
) {
    if let Some(mut timestamps) = timestamps {
        timestamps.submit(&device, &queue);
    }
}
