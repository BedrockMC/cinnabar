use crate::RuntimeStage;
use bevy::{
    core_pipeline::core_3d::graph::Node3d,
    render::render_graph::{InternedRenderLabel, RenderLabel},
};

/// The timed Core3d nodes; absent labels are skipped.
pub(super) fn timed_nodes() -> Vec<(InternedRenderLabel, RuntimeStage)> {
    use crate::ui_render::{UiOverlayLabel, UiWorldLabel, overlay::UiOverlayPostLabel};
    let mut nodes = vec![
        (Node3d::MainOpaquePass.intern(), RuntimeStage::GpuOpaque),
        (
            crate::chunk::TerrainPassLabel.intern(),
            RuntimeStage::GpuOpaque,
        ),
        (
            Node3d::MainTransparentPass.intern(),
            RuntimeStage::GpuTransparent,
        ),
        (UiWorldLabel.intern(), RuntimeStage::GpuUi),
        (UiOverlayLabel.intern(), RuntimeStage::GpuUi),
        (UiOverlayPostLabel.intern(), RuntimeStage::GpuUi),
        (
            crate::viewmodel_render::HandLabel.intern(),
            RuntimeStage::GpuHand,
        ),
        (
            crate::hand_rig_render::HandRigLabel.intern(),
            RuntimeStage::GpuHand,
        ),
        (Node3d::Tonemapping.intern(), RuntimeStage::GpuTonemapping),
        (Node3d::Fxaa.intern(), RuntimeStage::GpuFxaa),
    ];
    // Timestamps written just before presentation make macOS 26 flicker.
    if !cfg!(target_os = "macos") {
        nodes.push((Node3d::Upscaling.intern(), RuntimeStage::GpuBlit));
    }
    #[cfg(feature = "enhanced")]
    nodes.extend(crate::enhanced::graph::timed_nodes());
    nodes
}
