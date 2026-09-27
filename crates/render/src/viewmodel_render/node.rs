use super::*;
use bevy::{
    ecs::query::QueryItem,
    render::{
        render_graph::{NodeRunError, RenderGraphContext, ViewNode},
        renderer::RenderContext,
        view::ViewTarget,
    },
};
pub(super) struct HandViewNode;
impl ViewNode for HandViewNode {
    type ViewQuery = (
        &'static MainEntity,
        &'static ExtractedView,
        &'static ViewTarget,
        &'static Msaa,
    );
    fn run(
        &self,
        graph: &mut RenderGraphContext,
        context: &mut RenderContext,
        (owner, view, target, msaa): QueryItem<Self::ViewQuery>,
        world: &World,
    ) -> Result<(), NodeRunError> {
        let (Some(gpu), Some(gate), Some(drawn), Some(cache)) = (
            world.get_resource::<HandGpu>(),
            world.get_resource::<ViewmodelCompletionGate>(),
            world.get_resource::<HandDrawn>(),
            world.get_resource::<PipelineCache>(),
        ) else {
            return Ok(());
        };
        let Some(token) = gpu.token else {
            return Ok(());
        };
        if owner.id() != token.owner {
            return Ok(());
        }
        if view.hdr != token.hdr
            || msaa.samples() != token.samples
            || view.viewport != UVec4::new(0, 0, token.viewport[0], token.viewport[1])
        {
            gate.reject(token);
            return Ok(());
        }
        let color = target
            .sampled_main_texture()
            .unwrap_or_else(|| target.main_texture());
        let extent = color.size();
        if [extent.width, extent.height] != token.viewport || color.sample_count() != token.samples
        {
            gate.reject(token);
            return Ok(());
        }
        let (Some(depth), Some(vertices), Some(binding), Some(pipeline)) = (
            &gpu.depth,
            &gpu.vertices,
            &gpu.bind_group,
            gpu.pipeline.and_then(|id| cache.get_render_pipeline(id)),
        ) else {
            gate.reject(token);
            return Ok(());
        };
        let attachments = [Some(target.get_color_attachment())];
        let mut pass = context.begin_tracked_render_pass(RenderPassDescriptor {
            label: Some("camera-local neutral empty hand"),
            color_attachments: &attachments,
            depth_stencil_attachment: Some(RenderPassDepthStencilAttachment {
                view: &depth.view,
                depth_ops: Some(Operations {
                    load: LoadOp::Clear(0.0),
                    store: StoreOp::Discard,
                }),
                stencil_ops: None,
            }),
            timestamp_writes: None,
            occlusion_query_set: None,
        });
        pass.set_render_pipeline(pipeline);
        pass.set_bind_group(0, binding, &[]);
        pass.set_vertex_buffer(0, vertices.slice(..));
        pass.draw(0..gpu.vertex_count, 0..1);
        if let Some(scene) = world.get_resource::<ViewmodelScene>()
            && let Some(frame) = &scene.frame
            && frame.token == token
            && let Some((revision, first, page)) = frame.fallback
            && let Some(coverage) = world.get_resource::<crate::ui_render::UiHandCoverage>()
        {
            record_overlay_coverage(
                coverage,
                gate,
                token,
                graph.view_entity(),
                owner.id(),
                (revision, first, page),
            );
        }
        *drawn.0.lock().expect("hand drawn lock") = Some(token);
        Ok(())
    }
}

pub(super) fn record_overlay_coverage(
    coverage: &crate::ui_render::UiHandCoverage,
    gate: &ViewmodelCompletionGate,
    token: ViewmodelToken,
    view: Entity,
    main: Entity,
    fallback: (u64, u32, u32),
) {
    // Called only after the current complete hand range has been encoded.
    // Prior queue completion is necessary, not a presentation-success claim.
    if main == token.owner && gate.completed(token) {
        coverage.record(view, main, fallback.0, fallback.1, fallback.2);
    }
}
