//! The gamma-space UI layer: UI quads blend in an 8-bit sRGB-encoded offscreen
//! target, then one pass composites that layer over the scene in sRGB values,
//! as vanilla's UI blends, instead of in linear light.
use super::*;
use bevy::render::{
    render_resource::{
        BindGroup, BindGroupEntries, BindGroupLayout, Extent3d, LoadOp, Operations,
        RenderPassColorAttachment, RenderPassDescriptor, StoreOp, TextureDescriptor,
        TextureDimension, TextureUsages, TextureView,
    },
    renderer::RenderContext,
    texture::{CachedTexture, TextureCache},
};

pub(crate) const UI_COMPOSITE_SHADER_HANDLE: Handle<Shader> =
    uuid_handle!("f5b1f3c2-7a0e-4d0c-9c5e-3a8a4b1e6d21");
/// The UI layer's format: raw bytes, so blending happens on sRGB-encoded values.
pub(crate) const UI_LAYER_FORMAT: TextureFormat = TextureFormat::Rgba8Unorm;

/// A view's offscreen UI layer for this frame.
#[derive(Component)]
pub(crate) struct UiLayerTexture(pub(crate) CachedTexture);

pub(crate) fn prepare_ui_layers(
    mut commands: Commands,
    mut cache: ResMut<TextureCache>,
    device: Res<RenderDevice>,
    views: Query<(Entity, &ViewTarget)>,
) {
    for (entity, target) in &views {
        let size = target.main_texture().size();
        let texture = cache.get(
            &device,
            TextureDescriptor {
                label: Some("gamma-space UI layer"),
                size: Extent3d {
                    width: size.width,
                    height: size.height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: TextureDimension::D2,
                format: UI_LAYER_FORMAT,
                usage: TextureUsages::RENDER_ATTACHMENT | TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            },
        );
        commands.entity(entity).insert(UiLayerTexture(texture));
    }
}

#[derive(Resource)]
pub(crate) struct UiCompositePipeline {
    pub(crate) layout: BindGroupLayoutDescriptor,
    variants: Variants<RenderPipeline, UiCompositeSpecializer>,
}

struct UiCompositeSpecializer;

#[derive(Clone, Copy, PartialEq, Eq, Hash, SpecializerKey)]
pub(crate) struct UiCompositeKey {
    pub(crate) hdr: bool,
}

impl Specializer<RenderPipeline> for UiCompositeSpecializer {
    type Key = UiCompositeKey;

    fn specialize(
        &self,
        key: Self::Key,
        descriptor: &mut RenderPipelineDescriptor,
    ) -> Result<Canonical<Self::Key>, BevyError> {
        let target = descriptor.fragment.as_mut().unwrap().targets[0]
            .as_mut()
            .unwrap();
        target.format = if key.hdr {
            ViewTarget::TEXTURE_FORMAT_HDR
        } else {
            TextureFormat::bevy_default()
        };
        Ok(key)
    }
}

impl FromWorld for UiCompositePipeline {
    fn from_world(_world: &mut World) -> Self {
        let texture = |binding| BindGroupLayoutEntry {
            binding,
            visibility: ShaderStages::FRAGMENT,
            ty: BindingType::Texture {
                sample_type: TextureSampleType::Float { filterable: false },
                view_dimension: TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        };
        let layout =
            BindGroupLayoutDescriptor::new("UI composite layout", &[texture(0), texture(1)]);
        let descriptor = RenderPipelineDescriptor {
            label: Some("gamma-space UI composite".into()),
            layout: vec![layout.clone()],
            vertex: VertexState {
                shader: UI_COMPOSITE_SHADER_HANDLE,
                entry_point: Some("composite_vertex".into()),
                ..default()
            },
            fragment: Some(FragmentState {
                shader: UI_COMPOSITE_SHADER_HANDLE,
                entry_point: Some("composite_fragment".into()),
                targets: vec![Some(ColorTargetState {
                    format: TextureFormat::bevy_default(),
                    blend: None,
                    write_mask: ColorWrites::ALL,
                })],
                ..default()
            }),
            ..default()
        };
        Self {
            layout,
            variants: Variants::new(UiCompositeSpecializer, descriptor),
        }
    }
}

impl UiCompositePipeline {
    pub(crate) fn specialize(
        &mut self,
        cache: &PipelineCache,
        key: UiCompositeKey,
    ) -> Option<CachedRenderPipelineId> {
        self.variants.specialize(cache, key).ok()
    }
}

/// Composite `layer` over the view's scene into its next main texture.
pub(crate) fn composite(
    context: &mut RenderContext,
    target: &ViewTarget,
    layer: &TextureView,
    pipeline: &RenderPipeline,
    layout: &BindGroupLayout,
) {
    let write = target.post_process_write();
    let bind_group: BindGroup = context.render_device().create_bind_group(
        "UI composite bind group",
        layout,
        &BindGroupEntries::sequential((layer, write.source)),
    );
    let attachments = [Some(RenderPassColorAttachment {
        view: write.destination,
        depth_slice: None,
        resolve_target: None,
        ops: Operations {
            load: LoadOp::Clear(Default::default()),
            store: StoreOp::Store,
        },
    })];
    let mut pass = context.begin_tracked_render_pass(RenderPassDescriptor {
        label: Some("gamma-space UI composite"),
        color_attachments: &attachments,
        depth_stencil_attachment: None,
        timestamp_writes: None,
        occlusion_query_set: None,
    });
    pass.set_render_pipeline(pipeline);
    pass.set_bind_group(0, &bind_group, &[]);
    pass.draw(0..3, 0..1);
}
