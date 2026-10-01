//! Ordered depth-free HUD overlay and exact current-frame hand coverage.
use super::*;
use crate::ui::UI_BLEND_ALPHA;
use bevy::{
    camera::{MainPassResolutionOverride, Viewport},
    core_pipeline::core_3d::graph::{Core3d, Node3d},
    ecs::query::QueryItem,
    render::{
        camera::ExtractedCamera,
        render_graph::{
            NodeRunError, RenderGraph, RenderGraphContext, RenderLabel, ViewNode, ViewNodeRunner,
        },
        render_resource::RenderPassDescriptor,
        renderer::RenderContext,
    },
};
use std::{collections::BTreeMap, ops::Range, sync::Mutex};

#[derive(Debug, Clone, Hash, Eq, PartialEq, RenderLabel)]
pub(crate) struct UiOverlayLabel;
/// Per-frame draw encoding coverage, not queue completion or presentation.
/// The optional producer must independently require its prior completion gate.
#[derive(Default, Resource)]
pub(crate) struct UiHandCoverage(Mutex<HandCoverageState>);
#[derive(Default)]
struct HandCoverageState {
    epoch: u64,
    exhausted: bool,
    draw: Option<(u64, Entity, Entity, u64, u32, u32)>,
}
impl UiHandCoverage {
    pub(crate) fn clear(&self) {
        let mut state = self.0.lock().expect("HUD hand coverage lock");
        state.draw = None;
        if let Some(next) = state.epoch.checked_add(1) {
            state.epoch = next;
        } else {
            state.exhausted = true;
        }
    }
    pub(crate) fn record(&self, view: Entity, main: Entity, revision: u64, first: u32, page: u32) {
        let mut state = self.0.lock().expect("HUD hand coverage lock");
        if !state.exhausted {
            state.draw = Some((state.epoch, view, main, revision, first, page));
        }
    }
    pub(crate) fn range(
        &self,
        view: Entity,
        main: Entity,
        revision: Option<u64>,
        batches: &[UiRenderBatch],
        index_count: usize,
    ) -> Option<Range<u32>> {
        let state = self.0.lock().expect("HUD hand coverage lock");
        let (epoch, owner, main_owner, expected, first, page) = state.draw?;
        if state.exhausted
            || epoch != state.epoch
            || owner != view
            || main_owner != main
            || revision != Some(expected)
            || first % 3 != 0
        {
            return None;
        }
        let end = first.checked_add(6)?;
        if end as usize > index_count {
            return None;
        }
        let mut containing = batches.iter().filter(|b| {
            first >= b.first_index
                && b.first_index
                    .checked_add(b.index_count)
                    .is_some_and(|last| end <= last)
        });
        let batch = containing.next()?;
        if batch.texture_page != page || batch.blend_mode != UI_BLEND_ALPHA {
            return None;
        }
        if containing.next().is_some() {
            return None;
        }
        Some(first..end)
    }
}
pub(crate) fn retained_batch_ranges(
    batch: &UiRenderBatch,
    skip: Option<&Range<u32>>,
) -> [Option<Range<u32>>; 2] {
    let end = batch.first_index + batch.index_count;
    if let Some(skip) = skip
        && skip.start >= batch.first_index
        && skip.end <= end
    {
        [
            (batch.first_index < skip.start).then_some(batch.first_index..skip.start),
            (skip.end < end).then_some(skip.end..end),
        ]
    } else {
        [Some(batch.first_index..end), None]
    }
}
pub(crate) fn install_overlay_graph(world: &mut World) {
    let runner = ViewNodeRunner::<UiOverlayNode>::new(UiOverlayNode, world);
    let Some(mut graphs) = world.get_resource_mut::<RenderGraph>() else {
        return;
    };
    let Some(graph) = graphs.get_sub_graph_mut(Core3d) else {
        return;
    };
    if graph.get_node_state(UiOverlayLabel).is_err() {
        graph.add_node(UiOverlayLabel, runner);
    }
    graph.add_node_edges((
        Node3d::MainTransparentPass,
        UiOverlayLabel,
        Node3d::EndMainPass,
    ));
}

pub(super) fn queue_ui_overlay(
    pipeline_cache: Res<PipelineCache>,
    mut pipeline: ResMut<UiPipeline>,
    mut composite: ResMut<super::composite::UiCompositePipeline>,
    mut gpu: ResMut<UiGpu>,
    views: Query<(Entity, &ExtractedView, &Msaa)>,
    coverage: Option<Res<UiHandCoverage>>,
) {
    // Always clear the previous render-frame coverage, including empty UI and
    // unchanged accepted revisions, before any preparation/queue early return.
    if let Some(coverage) = coverage {
        coverage.clear();
    }
    // Retain unchanged view entries rather than freeing/reallocating tree nodes
    // every frame; only departed views release their cached pair.
    retain_view_pipeline_entries(&mut gpu.view_pipelines, |view| views.contains(view));
    gpu.composite_pipelines
        .retain(|view, _| views.contains(*view));
    if gpu.batches.is_empty()
        || gpu
            .textures
            .buckets
            .iter()
            .any(|bucket| bucket.bind_group.is_none())
        || gpu.vertex_buffer.is_none()
        || gpu.index_buffer.is_none()
    {
        return;
    }
    for (view_entity, view, msaa) in &views {
        let Ok(pipeline_id) = pipeline.variants.specialize(
            &pipeline_cache,
            UiPipelineKey {
                msaa: *msaa,
                hdr: view.hdr,
                invert_blend: false,
                layer: true,
            },
        ) else {
            gpu.view_pipelines.remove(&view_entity);
            continue;
        };
        let Ok(invert_pipeline_id) = pipeline.variants.specialize(
            &pipeline_cache,
            UiPipelineKey {
                msaa: *msaa,
                hdr: view.hdr,
                invert_blend: true,
                layer: false,
            },
        ) else {
            gpu.view_pipelines.remove(&view_entity);
            continue;
        };
        cache_view_pipeline_pair(
            &mut gpu.view_pipelines,
            view_entity,
            (pipeline_id, invert_pipeline_id),
        );
        match composite.specialize(
            &pipeline_cache,
            super::composite::UiCompositeKey { hdr: view.hdr },
        ) {
            Some(id) => {
                gpu.composite_pipelines.insert(view_entity, id);
            }
            None => {
                gpu.composite_pipelines.remove(&view_entity);
            }
        }
    }
}

pub(crate) fn retain_view_pipeline_entries(
    entries: &mut BTreeMap<Entity, (CachedRenderPipelineId, CachedRenderPipelineId)>,
    mut live: impl FnMut(Entity) -> bool,
) {
    entries.retain(|view, _| live(*view));
}
pub(crate) fn cache_view_pipeline_pair(
    entries: &mut BTreeMap<Entity, (CachedRenderPipelineId, CachedRenderPipelineId)>,
    view: Entity,
    pair: (CachedRenderPipelineId, CachedRenderPipelineId),
) {
    if let Some(existing) = entries.get_mut(&view) {
        *existing = pair;
    } else {
        entries.insert(view, pair);
    }
}
pub(crate) fn overlay_pipeline_pair<'a, T>(
    batches: &[UiRenderBatch],
    entries: &'a BTreeMap<Entity, T>,
    view: Entity,
) -> Option<&'a T> {
    // Empty UI can retain an old format/sample cache pair, but must never bind
    // it against a changed target, even in a pass with zero draw commands.
    if batches.is_empty() {
        None
    } else {
        entries.get(&view)
    }
}

struct UiOverlayNode;
impl ViewNode for UiOverlayNode {
    type ViewQuery = (
        &'static ViewTarget,
        &'static MainEntity,
        &'static ExtractedCamera,
        Option<&'static MainPassResolutionOverride>,
        Option<&'static super::composite::UiLayerTexture>,
    );
    fn run(
        &self,
        graph: &mut RenderGraphContext,
        context: &mut RenderContext,
        (target, main, camera, resolution_override, layer): QueryItem<Self::ViewQuery>,
        world: &World,
    ) -> Result<(), NodeRunError> {
        let (Some(gpu), Some(pipeline_cache), Some(composite)) = (
            world.get_resource::<UiGpu>(),
            world.get_resource::<PipelineCache>(),
            world.get_resource::<super::composite::UiCompositePipeline>(),
        ) else {
            return Ok(());
        };
        let (Some(vertices), Some(indices), Some((alpha, invert)), Some(layer)) = (
            &gpu.vertex_buffer,
            &gpu.index_buffer,
            overlay_pipeline_pair(&gpu.batches, &gpu.view_pipelines, graph.view_entity()),
            layer,
        ) else {
            return Ok(());
        };
        if gpu.textures.buckets.len() != gpu.textures.allocated_buckets().len()
            || gpu
                .textures
                .buckets
                .iter()
                .any(|bucket| bucket.bind_group.is_none())
        {
            return Ok(());
        }
        let Some(batches) = resolved_batches(
            gpu.accepted_revision,
            &gpu.batches,
            &gpu.textures.locations,
            gpu.textures.allocated_buckets(),
        ) else {
            return Ok(());
        };
        let (Some(layer_pipeline), Some(composite_pipeline)) = (
            pipeline_cache.get_render_pipeline(*alpha),
            gpu.composite_pipelines
                .get(&graph.view_entity())
                .and_then(|id| pipeline_cache.get_render_pipeline(*id)),
        ) else {
            return Ok(());
        };
        let composite_layout = pipeline_cache.get_bind_group_layout(&composite.layout);
        let viewport = overlay_viewport(camera.viewport.as_ref(), resolution_override);
        let skip = world.get_resource::<UiHandCoverage>().and_then(|coverage| {
            coverage.range(
                graph.view_entity(),
                main.id(),
                gpu.accepted_revision,
                &gpu.batches,
                gpu.index_count,
            )
        });
        let batches: Vec<_> = batches.collect();
        // Alpha batches blend in the gamma-space layer; an invert batch (the
        // crosshair) must see the scene, so the layer composites before it.
        for segment in batches.split_inclusive(|(_, batch, _)| batch.blend_mode == UI_BLEND_INVERT)
        {
            let (layered, inverted) = match segment.split_last() {
                Some((last, rest)) if last.1.blend_mode == UI_BLEND_INVERT => (rest, Some(last)),
                _ => (segment, None),
            };
            if !layered.is_empty() {
                let attachments = [Some(
                    bevy::render::render_resource::RenderPassColorAttachment {
                        view: &layer.0.default_view,
                        depth_slice: None,
                        resolve_target: None,
                        ops: bevy::render::render_resource::Operations {
                            load: bevy::render::render_resource::LoadOp::Clear(Default::default()),
                            store: bevy::render::render_resource::StoreOp::Store,
                        },
                    },
                )];
                let mut pass = context.begin_tracked_render_pass(RenderPassDescriptor {
                    label: Some("gamma-space UI layer"),
                    color_attachments: &attachments,
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                });
                pass.set_render_pipeline(layer_pipeline);
                draw_batches(
                    &mut pass,
                    gpu,
                    vertices,
                    indices,
                    viewport.as_ref(),
                    layered,
                    skip.as_ref(),
                );
                drop(pass);
                super::composite::composite(
                    context,
                    target,
                    &layer.0.default_view,
                    composite_pipeline,
                    &composite_layout,
                );
            }
            if let Some(inverted) = inverted {
                let Some(pipeline) = pipeline_cache.get_render_pipeline(*invert) else {
                    // Still compiling: skip the crosshair rather than blend it wrong.
                    continue;
                };
                let attachments = [Some(target.get_color_attachment())];
                let mut pass = context.begin_tracked_render_pass(RenderPassDescriptor {
                    label: Some("retained depth-free HUD invert"),
                    color_attachments: &attachments,
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                });
                pass.set_render_pipeline(pipeline);
                draw_batches(
                    &mut pass,
                    gpu,
                    vertices,
                    indices,
                    viewport.as_ref(),
                    std::slice::from_ref(inverted),
                    skip.as_ref(),
                );
            }
        }
        Ok(())
    }
}

/// Draw `batches` into `pass`, each under its own scissor and page bind group.
fn draw_batches<'w>(
    pass: &mut bevy::render::render_phase::TrackedRenderPass<'w>,
    gpu: &'w UiGpu,
    vertices: &'w Buffer,
    indices: &'w Buffer,
    viewport: Option<&Viewport>,
    batches: &[(usize, &UiRenderBatch, crate::UiTextureLocation)],
    skip: Option<&Range<u32>>,
) {
    if let Some(viewport) = viewport {
        pass.set_camera_viewport(viewport);
    }
    pass.set_vertex_buffer(0, vertices.slice(..));
    pass.set_index_buffer(indices.slice(..), wgpu::IndexFormat::Uint32);
    for (_, batch, location) in batches {
        let binding = gpu.textures.buckets[location.bucket]
            .bind_group
            .as_ref()
            .unwrap();
        pass.set_bind_group(0, binding, &[]);
        let scissor = batch.scissor;
        pass.set_scissor_rect(scissor.x, scissor.y, scissor.width, scissor.height);
        for range in retained_batch_ranges(batch, skip).into_iter().flatten() {
            pass.draw_indexed(range, 0, location.layer..location.layer + 1);
        }
    }
    pass.set_scissor_rect(0, 0, gpu.viewport_size[0], gpu.viewport_size[1]);
}

pub(crate) fn overlay_viewport(
    viewport: Option<&Viewport>,
    resolution_override: Option<&MainPassResolutionOverride>,
) -> Option<Viewport> {
    Viewport::from_viewport_and_override(viewport, resolution_override)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retained_hand_split_keeps_mixed_bucket_layer_blend_and_shadow_fill_order() {
        let plan =
            crate::UiTexturePlan::new(&[[1024, 1024], [2048, 2048], [256, 256], [2048, 2048]])
                .unwrap();
        let batches = [1, 0, 3, 2, 1]
            .into_iter()
            .enumerate()
            .map(|(index, page)| {
                UiRenderBatch::new(
                    page,
                    crate::ui::UiScissor::new(index as u32, 0, 20, 20),
                    index as u32 * 18,
                    18,
                    if index == 2 {
                        UI_BLEND_INVERT
                    } else {
                        UI_BLEND_ALPHA
                    },
                )
            })
            .collect::<Vec<_>>();
        let trace = resolved_batches(Some(7), &batches, plan.locations(), plan.buckets())
            .unwrap()
            .flat_map(|(index, batch, location)| {
                retained_batch_ranges(batch, Some(&(60..66)))
                    .into_iter()
                    .flatten()
                    .map(move |range| {
                        (
                            index,
                            location.bucket,
                            location.layer,
                            batch.blend_mode,
                            batch.scissor.x,
                            range,
                        )
                    })
            })
            .collect::<Vec<_>>();
        assert_eq!(
            trace,
            vec![
                (0, 1, 0, UI_BLEND_ALPHA, 0, 0..18),
                (1, 0, 0, UI_BLEND_ALPHA, 1, 18..36),
                (2, 1, 1, UI_BLEND_INVERT, 2, 36..54),
                (3, 2, 0, UI_BLEND_ALPHA, 3, 54..60),
                (3, 2, 0, UI_BLEND_ALPHA, 3, 66..72),
                (4, 1, 0, UI_BLEND_ALPHA, 4, 72..90),
            ]
        );
        let full = resolved_batches(Some(7), &batches, plan.locations(), plan.buckets())
            .unwrap()
            .flat_map(|(_, batch, _)| retained_batch_ranges(batch, None).into_iter().flatten())
            .collect::<Vec<_>>();
        assert_eq!(full, vec![0..18, 18..36, 36..54, 54..72, 72..90]);
        assert!(resolved_batches(None, &batches, plan.locations(), plan.buckets()).is_none());
        let mut locations = plan.locations().to_vec();
        locations[1].layer = 2;
        assert!(
            resolved_batches(Some(7), &batches, &locations, plan.buckets()).is_none(),
            "late invalid physical layer must emit no prefix"
        );
    }
}
