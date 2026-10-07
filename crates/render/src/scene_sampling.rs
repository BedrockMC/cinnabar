//! Single-sample consumers of the scene retain explicit MSAA resolve boundaries.

use bevy::render::{
    render_resource::{Texture, TextureView, TextureViewId},
    renderer::{RenderContext, RenderDevice},
    view::{ViewDepthTexture, ViewTarget},
};

#[cfg(test)]
mod tests;

/// A retained fullscreen copy into a matching single- or multisample colour attachment.
pub(crate) struct SceneCopy {
    pipeline: wgpu::RenderPipeline,
    sources: [(TextureViewId, wgpu::BindGroup); 2],
    samples: u32,
}

impl SceneCopy {
    /// Keeps both ping-pong scene inputs bound until a target is replaced.
    pub(crate) fn new(device: &RenderDevice, target: &ViewTarget, samples: u32) -> Self {
        Self::from_views(
            device,
            [target.main_texture_view(), target.main_texture_other_view()],
            target.main_texture_format(),
            samples,
        )
    }

    /// Builds one copy variant and the bounded set of source bindings it can use.
    fn from_views(
        device: &RenderDevice,
        views: [&TextureView; 2],
        format: wgpu::TextureFormat,
        samples: u32,
    ) -> Self {
        let device = device.wgpu_device();
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("resolved scene copy"),
            source: wgpu::ShaderSource::Wgsl(include_str!("scene_copy.wgsl").into()),
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("resolved scene copy"),
            layout: None,
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vertex"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fragment"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: Default::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState {
                count: samples,
                ..Default::default()
            },
            multiview: None,
            cache: None,
        });
        let layout = pipeline.get_bind_group_layout(0);
        let sources = views.map(|view| {
            (
                view.id(),
                device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("resolved scene copy input"),
                    layout: &layout,
                    entries: &[wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(view),
                    }],
                }),
            )
        });
        Self {
            pipeline,
            sources,
            samples,
        }
    }

    /// A changed ping-pong pair or sample count requires new retained bindings.
    pub(crate) fn matches(&self, target: &ViewTarget, samples: u32) -> bool {
        self.samples == samples
            && self
                .sources
                .iter()
                .any(|(id, _)| *id == target.main_texture_view().id())
            && self
                .sources
                .iter()
                .any(|(id, _)| *id == target.main_texture_other_view().id())
    }

    /// Copies resolved pixels without filtering and optionally resolves the new attachment.
    pub(crate) fn draw(
        &self,
        context: &mut RenderContext,
        source: &TextureView,
        destination: &TextureView,
        resolve: Option<&TextureView>,
        rect: Option<[u32; 4]>,
    ) {
        let binding = &self
            .sources
            .iter()
            .find(|(id, _)| *id == source.id())
            .expect("scene copy source must be prepared")
            .1;
        let mut pass = context
            .command_encoder()
            .begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("resolved scene MSAA writeback"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: destination,
                    depth_slice: None,
                    resolve_target: resolve.map(|view| &**view),
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
        if let Some([x0, y0, x1, y1]) = rect {
            pass.set_scissor_rect(x0, y0, x1 - x0, y1 - y0);
        }
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, binding, &[]);
        pass.draw(0..3, 0..1);
    }

    /// Restores the resolved scene before later geometry writes to Bevy's MSAA attachment.
    pub(crate) fn writeback(
        &self,
        context: &mut RenderContext,
        target: &ViewTarget,
        rect: Option<[u32; 4]>,
    ) {
        let Some(destination) = target.sampled_main_texture_view() else {
            return;
        };
        // The resolved target already holds the result; later geometry performs the next resolve.
        self.draw(context, target.main_texture_view(), destination, None, rect);
    }
}

/// Nearest reverse-Z surface for effects that require a single-sample depth texture.
pub(crate) struct ResolvedDepth {
    pub(crate) _texture: Texture,
    pub(crate) view: TextureView,
    source: TextureViewId,
    pipeline: wgpu::RenderPipeline,
    binding: wgpu::BindGroup,
}

impl ResolvedDepth {
    /// Samples depth instead of copying it, which also works for multisampled attachments.
    pub(crate) fn new(device: &RenderDevice, depth: &ViewDepthTexture) -> Self {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("resolved scene depth"),
            size: depth.texture.size(),
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Depth32Float,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = texture.create_view(&Default::default());
        let gpu = device.wgpu_device();
        let shader = gpu.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("resolved scene depth"),
            source: wgpu::ShaderSource::Wgsl(include_str!("scene_depth.wgsl").into()),
        });
        let multi = depth.texture.sample_count() > 1;
        let pipeline = gpu.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("resolved scene depth"),
            layout: None,
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vertex"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some(if multi { "multisampled" } else { "single" }),
                compilation_options: Default::default(),
                targets: &[],
            }),
            primitive: Default::default(),
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: true,
                depth_compare: wgpu::CompareFunction::Always,
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: Default::default(),
            multiview: None,
            cache: None,
        });
        let binding = gpu.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("resolved scene depth input"),
            layout: &pipeline.get_bind_group_layout(0),
            entries: &[wgpu::BindGroupEntry {
                binding: u32::from(multi),
                resource: wgpu::BindingResource::TextureView(depth.view()),
            }],
        });
        Self {
            _texture: texture,
            view,
            source: depth.view().id(),
            pipeline,
            binding,
        }
    }

    /// The source view identity includes both the attachment size and its sample count.
    pub(crate) fn matches(&self, depth: &ViewDepthTexture) -> bool {
        self.source == depth.view().id()
    }

    /// Writes the nearest covered surface; Hi-Z uses its separate conservative farthest resolve.
    pub(crate) fn draw(&self, context: &mut RenderContext, rect: Option<[u32; 4]>) {
        let mut pass = context
            .command_encoder()
            .begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("scene depth resolve"),
                color_attachments: &[],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &self.view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(0.0),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
            });
        if let Some([x0, y0, x1, y1]) = rect {
            pass.set_scissor_rect(x0, y0, x1 - x0, y1 - y0);
        }
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.binding, &[]);
        pass.draw(0..3, 0..1);
    }
}
