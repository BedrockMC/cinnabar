//! Device-bounded immutable neutral artwork, replaced whole when its identity changes.
use super::*;
use crate::actor::{
    ActorArtworkPages, MAX_ACTOR_GPU_PIXEL_BYTES, MAX_ACTOR_TEXTURE_PAGES, gpu::ActorDrawSpan,
};

pub(super) struct GpuArtworkPage {
    _texture: Texture,
    pub view: TextureView,
    pub bind_group: Option<BindGroup>,
}

#[derive(Default)]
pub(super) struct GpuArtwork {
    identity: Option<([u8; 32], [u8; 32])>,
    pub pages: Vec<GpuArtworkPage>,
    rejected: bool,
}

impl GpuArtwork {
    pub fn prepare(
        &mut self,
        pages: &ActorArtworkPages,
        device: &RenderDevice,
        queue: &RenderQueue,
    ) -> bool {
        let identity = (pages.identity, pages.entity_identity);
        if self.identity == Some(identity) {
            return !self.rejected;
        }
        if self.identity.take().is_some() {
            // Session packs change the artwork. wgpu keeps dropped pages alive until
            // work already submitted with them completes, so a generation is only
            // briefly doubled.
            self.pages.clear();
            self.rejected = false;
        }
        if pages.identity == [0; 32] {
            return true;
        }
        self.identity = Some(identity);
        let limits = device.limits();
        let bytes = pages.pages.iter().try_fold(
            crate::actor::MAX_RENDERED_PLAYERS * STANDARD_SKIN_BYTES,
            |total, page| total.checked_add(page.rgba8.len()),
        );
        if pages.pages.len() + 1 > MAX_ACTOR_TEXTURE_PAGES
            || bytes.is_none_or(|bytes| bytes > MAX_ACTOR_GPU_PIXEL_BYTES)
            || pages
                .pages
                .iter()
                .any(|page| page.layers > limits.max_texture_array_layers)
        {
            self.rejected = true;
            bevy::log::warn!(
                "neutral actor artwork exceeds device limits; generic artwork unavailable"
            );
            return false;
        }
        for page in pages.pages.iter() {
            // UVs are normalised, so a page past the device limit draws downscaled, not blank.
            let page = &page.fit_within(limits.max_texture_dimension_2d);
            let texture = device.create_texture_with_data(
                queue,
                &TextureDescriptor {
                    label: Some("immutable neutral binary-alpha actor page"),
                    size: Extent3d {
                        width: u32::from(page.width),
                        height: u32::from(page.height),
                        depth_or_array_layers: page.layers,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: TextureDimension::D2,
                    format: TextureFormat::Rgba8UnormSrgb,
                    usage: TextureUsages::TEXTURE_BINDING | TextureUsages::COPY_DST,
                    view_formats: &[],
                },
                TextureDataOrder::LayerMajor,
                &page.rgba8,
            );
            let view = texture.create_view(&TextureViewDescriptor {
                dimension: Some(TextureViewDimension::D2Array),
                ..default()
            });
            self.pages.push(GpuArtworkPage {
                _texture: texture,
                view,
                bind_group: None,
            });
        }
        true
    }

    pub fn invalidate_bindings(&mut self) {
        for page in &mut self.pages {
            page.bind_group = None;
        }
    }
}

pub(super) fn draw_spans(pages: &[u8]) -> Vec<ActorDrawSpan> {
    let mut spans: Vec<ActorDrawSpan> = Vec::new();
    for (index, page) in pages.iter().copied().enumerate() {
        if let Some(span) = spans.last_mut().filter(|span| span.page == page) {
            span.count += 1;
        } else {
            spans.push(ActorDrawSpan {
                page,
                first: index as u32,
                count: 1,
            });
        }
    }
    spans
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn page_spans_preserve_global_instance_order_and_bone_bases() {
        let spans = draw_spans(&[0, 1, 1, 0, 2]);
        assert_eq!(
            spans,
            vec![
                ActorDrawSpan {
                    page: 0,
                    first: 0,
                    count: 1
                },
                ActorDrawSpan {
                    page: 1,
                    first: 1,
                    count: 2
                },
                ActorDrawSpan {
                    page: 0,
                    first: 3,
                    count: 1
                },
                ActorDrawSpan {
                    page: 2,
                    first: 4,
                    count: 1
                },
            ]
        );
    }

    // A tall flipbook past the device limit draws downscaled instead of blanking every page.
    #[test]
    fn a_page_past_the_device_limit_uploads_downscaled() {
        use bevy::render::renderer::WgpuWrapper;
        use std::sync::Arc;
        let (device, queue) = wgpu::Device::noop(&wgpu::DeviceDescriptor::default());
        let device = RenderDevice::from(device);
        let queue = RenderQueue(Arc::new(WgpuWrapper::new(queue)));
        let side = device.limits().max_texture_dimension_2d;
        let height = u16::try_from(side * 2).unwrap();
        let mut pages = ActorArtworkPages::default();
        pages.identity = [1; 32];
        pages.pages = Arc::from([crate::actor::ActorTexturePage {
            width: 2,
            height,
            layers: 1,
            rgba8: vec![255; 2 * usize::from(height) * 4].into(),
        }]);
        let mut gpu = GpuArtwork::default();
        assert!(gpu.prepare(&pages, &device, &queue));
        assert_eq!(gpu.pages.len(), 1);
    }

    #[test]
    fn replacement_artwork_supersedes_the_previous_generation() {
        use bevy::render::renderer::WgpuWrapper;
        use std::sync::Arc;
        let (device, queue) = wgpu::Device::noop(&wgpu::DeviceDescriptor::default());
        let device = RenderDevice::from(device);
        let queue = RenderQueue(Arc::new(WgpuWrapper::new(queue)));
        let mut pages = ActorArtworkPages::default();
        pages.identity = [1; 32];
        pages.entity_identity = [2; 32];
        pages.pages = Arc::from([crate::actor::ActorTexturePage {
            width: 16,
            height: 16,
            layers: 1,
            rgba8: vec![255; 16 * 16 * 4].into(),
        }]);
        let mut gpu = GpuArtwork::default();
        assert!(gpu.prepare(&pages, &device, &queue));
        assert_eq!(gpu.pages.len(), 1);
        assert!(gpu.prepare(&pages, &device, &queue));
        pages.entity_identity = [3; 32];
        assert!(gpu.prepare(&pages, &device, &queue));
        assert_eq!(gpu.pages.len(), 1);
        assert_eq!(gpu.identity, Some(([1; 32], [3; 32])));
        pages.identity = [0; 32];
        assert!(gpu.prepare(&pages, &device, &queue));
        assert!(gpu.pages.is_empty() && gpu.identity.is_none());
    }
}
