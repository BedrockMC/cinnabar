//! Retained dimension buckets and transactional dirty-page write planning.

use bevy::render::{
    render_resource::{
        BindGroup, Extent3d, Origin3d, TexelCopyBufferLayout, TexelCopyTextureInfo, Texture,
        TextureDescriptor, TextureDimension, TextureFormat, TextureUsages, TextureView,
        TextureViewDescriptor, TextureViewDimension,
    },
    renderer::{RenderDevice, RenderQueue},
};

use crate::ui::UiRenderRejectReason;
use crate::{UiTextureCatalog, UiTextureLocation, UiTexturePage, UiTexturePlan};

/// Observes schedule-separated device-resource changes, not arbitrary context IDs.
pub(crate) struct DeviceObservation {
    last_observed: bevy::ecs::change_detection::Tick,
    invalidated: bool,
}
impl DeviceObservation {
    pub(crate) fn new(now: bevy::ecs::change_detection::Tick) -> Self {
        Self {
            last_observed: now,
            invalidated: false,
        }
    }
    pub(crate) fn observe(
        &mut self,
        changed: bevy::ecs::change_detection::Tick,
        now: bevy::ecs::change_detection::Tick,
        same_device: bool,
    ) -> bool {
        let gap = now.get().wrapping_sub(self.last_observed.get());
        self.invalidated |= !same_device
            || gap >= bevy::ecs::change_detection::MAX_CHANGE_AGE
            || changed.is_newer_than(self.last_observed, now);
        self.last_observed = now;
        !self.invalidated
    }
}

pub(super) struct GpuBucket {
    pub(super) texture: Texture,
    pub(super) view: TextureView,
    pub(super) bind_group: Option<BindGroup>,
}

#[derive(Default)]
pub(super) struct TextureUploadState {
    static_identity: Option<[u8; 32]>,
    plan: Option<UiTexturePlan>,
    uploaded: Vec<[u8; 32]>,
}

impl TextureUploadState {
    fn dirty(&self, catalog: &UiTextureCatalog) -> Result<Vec<usize>, UiRenderRejectReason> {
        if self
            .static_identity
            .is_some_and(|id| id != catalog.static_identity())
            || self
                .plan
                .as_ref()
                .is_some_and(|plan| plan != catalog.plan())
        {
            return Err(UiRenderRejectReason::TextureIdentityConflict {
                identity: catalog.static_identity(),
            });
        }
        Ok(catalog
            .pages()
            .iter()
            .enumerate()
            .filter_map(|(index, page)| {
                (self.uploaded.get(index) != Some(&page.identity())).then_some(index)
            })
            .collect())
    }

    /// The executor is shared by device preparation and deterministic recording
    /// tests. Only successful issuance of ALL writes commits content IDs.
    fn execute<E>(
        &mut self,
        catalog: &UiTextureCatalog,
        dirty: &[usize],
        mut write: impl FnMut(usize, &UiTexturePage, UiTextureLocation) -> Result<(), E>,
    ) -> Result<(), E> {
        for &index in dirty {
            write(
                index,
                &catalog.pages()[index],
                catalog.plan().locations()[index],
            )?;
        }
        self.uploaded.clear();
        self.uploaded
            .extend(catalog.pages().iter().map(UiTexturePage::identity));
        self.static_identity = Some(catalog.static_identity());
        if self.plan.is_none() {
            self.plan = Some(catalog.plan().clone());
        }
        Ok(())
    }
}

#[derive(Default)]
pub(super) struct UiGpuTextures {
    pub(super) buckets: Vec<GpuBucket>,
    pub(super) state: TextureUploadState,
    pub(super) locations: Vec<UiTextureLocation>,
    pub(super) bytes: usize,
    allocation_identity: Option<[u8; 32]>,
    allocation_plan: Option<UiTexturePlan>,
}

impl UiGpuTextures {
    pub(super) fn allocated_buckets(&self) -> &[crate::UiTextureBucket] {
        self.allocation_plan
            .as_ref()
            .map_or(&[], |plan| plan.buckets())
    }
    pub(super) fn resident(&self, catalog: &UiTextureCatalog) -> bool {
        self.allocation_identity == Some(catalog.static_identity())
            && self.allocation_plan.as_ref() == Some(catalog.plan())
            && self.buckets.len() == catalog.plan().buckets().len()
            && self.locations == catalog.plan().locations()
            && self.state.uploaded.len() == catalog.pages().len()
            && self
                .state
                .uploaded
                .iter()
                .zip(catalog.pages())
                .all(|(id, page)| *id == page.identity())
    }
    pub(super) fn prepare(
        &mut self,
        catalog: &UiTextureCatalog,
        device: &RenderDevice,
        queue: &RenderQueue,
    ) -> Result<(), UiRenderRejectReason> {
        let limits = device.limits();
        catalog.plan().validate_device(
            limits.max_texture_dimension_2d,
            limits.max_texture_array_layers,
        )?;
        if self
            .allocation_identity
            .is_some_and(|id| id != catalog.static_identity())
            || self
                .allocation_plan
                .as_ref()
                .is_some_and(|plan| plan != catalog.plan())
        {
            return Err(UiRenderRejectReason::TextureIdentityConflict {
                identity: catalog.static_identity(),
            });
        }
        if self.allocation_identity.is_some()
            && (self.buckets.len() != catalog.plan().buckets().len()
                || self.locations != catalog.plan().locations())
        {
            return Err(UiRenderRejectReason::InvalidTextureExtent);
        }
        let dirty = self.state.dirty(catalog)?;
        let format = TextureFormat::Rgba8UnormSrgb.guaranteed_format_features(device.features());
        if !format
            .allowed_usages
            .contains(TextureUsages::TEXTURE_BINDING | TextureUsages::COPY_DST)
            || !format
                .flags
                .contains(wgpu::TextureFormatFeatureFlags::FILTERABLE)
        {
            return Err(UiRenderRejectReason::InvalidTextureExtent);
        }
        // All catalog and per-device admission checks precede allocation/writes.
        if self.buckets.is_empty() {
            self.allocation_identity = Some(catalog.static_identity());
            self.allocation_plan = Some(catalog.plan().clone());
            for bucket in catalog.plan().buckets() {
                let texture = device.create_texture(&TextureDescriptor {
                    label: Some("bounded UI dimension bucket"),
                    size: Extent3d {
                        width: bucket.dimensions[0],
                        height: bucket.dimensions[1],
                        depth_or_array_layers: bucket.layers,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: TextureDimension::D2,
                    format: TextureFormat::Rgba8UnormSrgb,
                    usage: TextureUsages::TEXTURE_BINDING | TextureUsages::COPY_DST,
                    view_formats: &[],
                });
                let view = texture.create_view(&TextureViewDescriptor {
                    label: Some("bounded UI dimension bucket view"),
                    dimension: Some(TextureViewDimension::D2Array),
                    ..Default::default()
                });
                self.buckets.push(GpuBucket {
                    texture,
                    view,
                    bind_group: None,
                });
            }
            self.locations = catalog.plan().locations().to_vec();
            self.bytes = catalog.plan().bytes();
        }
        if self.buckets.len() != catalog.plan().buckets().len() {
            return Err(UiRenderRejectReason::InvalidTextureExtent);
        }
        if dirty.is_empty() {
            return Ok(());
        }
        let buckets = &self.buckets;
        self.state.execute(catalog, &dirty, |_, page, location| {
            let [width, height] = page.dimensions();
            queue.write_texture(
                TexelCopyTextureInfo {
                    texture: &buckets[location.bucket].texture,
                    mip_level: 0,
                    origin: Origin3d {
                        x: 0,
                        y: 0,
                        z: location.layer,
                    },
                    aspect: Default::default(),
                },
                page.pixels(),
                TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(width * 4),
                    rows_per_image: Some(height),
                },
                Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
            );
            Ok::<(), UiRenderRejectReason>(())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn device_observation_handles_clamped_aging_wrap_and_unknown_gaps() {
        use bevy::ecs::change_detection::{MAX_CHANGE_AGE, Tick};
        let mut observation = DeviceObservation::new(Tick::new(10));
        assert!(observation.observe(Tick::new(1), Tick::new(11), true));
        let near_age = MAX_CHANGE_AGE - 1;
        assert!(
            observation.observe(Tick::new(1), Tick::new(11 + near_age), true),
            "old unchanged resource aging is not a replacement"
        );
        assert!(
            observation.observe(Tick::new(12), Tick::new(12 + near_age), true),
            "clamped old resource stamp is not newer than last observation"
        );
        let mut wrapped = DeviceObservation::new(Tick::new(u32::MAX - 2));
        assert!(wrapped.observe(Tick::new(u32::MAX - 5), Tick::new(1), true));
        assert!(!wrapped.observe(Tick::new(2), Tick::new(3), true));
        assert!(
            !wrapped.observe(Tick::new(0), Tick::new(4), true),
            "observed invalidation is permanent"
        );
        let mut missing = DeviceObservation::new(Tick::new(10));
        assert!(
            !missing.observe(Tick::new(1), Tick::new(10 + MAX_CHANGE_AGE), true),
            "unknown detection-window gap fails closed"
        );
        assert!(!missing.observe(Tick::new(1), Tick::new(11 + MAX_CHANGE_AGE), true));
        assert!(
            !DeviceObservation::new(Tick::new(10)).observe(
                Tick::new(1),
                Tick::new(11 + MAX_CHANGE_AGE),
                true
            ),
            "gap beyond detection window is also unknown"
        );
        let mut mismatch = DeviceObservation::new(Tick::new(1));
        assert!(!mismatch.observe(Tick::new(1), Tick::new(2), false));
        assert!(!mismatch.observe(Tick::new(1), Tick::new(3), true));
        assert!(
            DeviceObservation::new(Tick::new(3)).observe(Tick::new(1), Tick::new(4), true),
            "actual recreation starts a new observer"
        );
    }

    fn catalog(value: u8) -> UiTextureCatalog {
        UiTextureCatalog::new(
            vec![
                UiTexturePage::owned([1024, 1024], vec![255; 1024 * 1024 * 4].into()).unwrap(),
                UiTexturePage::owned([256, 256], vec![value; 256 * 256 * 4].into()).unwrap(),
            ],
            1,
        )
        .unwrap()
    }

    #[test]
    fn recording_executor_writes_only_changed_layers_and_retries_refusal() {
        let base = catalog(0);
        let mut state = TextureUploadState::default();
        let first = state.dirty(&base).unwrap();
        assert_eq!(first, [0, 1]);
        state
            .execute(&base, &first, |_, _, _| Ok::<_, ()>(()))
            .unwrap();
        assert!(state.dirty(&base).unwrap().is_empty());
        for value in 1..=100 {
            let changed = base
                .replace_dynamic(vec![
                    UiTexturePage::owned([256, 256], vec![value; 256 * 256 * 4].into()).unwrap(),
                ])
                .unwrap();
            let dirty = state.dirty(&changed).unwrap();
            assert_eq!(dirty, [1]);
            assert!(state.execute(&changed, &dirty, |_, _, _| Err(())).is_err());
            assert_eq!(state.dirty(&changed).unwrap(), [1]);
            let mut written = Vec::new();
            state
                .execute(&changed, &dirty, |i, page, location| {
                    written.push((i, page.pixels().len(), location));
                    Ok::<_, ()>(())
                })
                .unwrap();
            assert_eq!(written.len(), 1);
            assert_eq!(written[0].1, 256 * 256 * 4);
            assert!(state.dirty(&changed).unwrap().is_empty());
        }
        assert!(state.dirty(&catalog(101)).is_ok());
        let different_static = UiTextureCatalog::new(
            vec![
                UiTexturePage::owned([1024, 1024], vec![0; 1024 * 1024 * 4].into()).unwrap(),
                base.pages()[1].clone(),
            ],
            1,
        )
        .unwrap();
        assert!(state.dirty(&different_static).is_err());
    }

    #[test]
    fn partial_initial_issuance_never_commits_and_retries_all_pages() {
        let catalog = catalog(0);
        let mut state = TextureUploadState::default();
        let dirty = state.dirty(&catalog).unwrap();
        assert!(
            state
                .execute(&catalog, &dirty, |i, _, _| if i == 1 {
                    Err(())
                } else {
                    Ok(())
                })
                .is_err()
        );
        assert!(state.static_identity.is_none());
        assert_eq!(state.dirty(&catalog).unwrap(), [0, 1]);
    }

    #[test]
    fn missing_bucket_and_new_generation_never_recreate_or_reuse_uploaded_ids() {
        let (device, queue) = wgpu::Device::noop(&wgpu::DeviceDescriptor::default());
        let device = RenderDevice::from(device);
        let queue = RenderQueue(std::sync::Arc::new(
            bevy::render::renderer::WgpuWrapper::new(queue),
        ));
        let catalog = catalog(0);
        let mut gpu = UiGpuTextures::default();
        gpu.prepare(&catalog, &device, &queue).unwrap();
        assert_eq!(gpu.buckets.len(), 2);
        assert!(gpu.state.dirty(&catalog).unwrap().is_empty());
        assert!(gpu.resident(&catalog));
        let changed_generation =
            UiTextureCatalog::with_source_identity(catalog.pages().to_vec(), 1, [7; 32]).unwrap();
        assert!(gpu.prepare(&changed_generation, &device, &queue).is_err());
        assert_eq!(gpu.buckets.len(), 2);
        gpu.buckets.pop();
        assert!(!gpu.resident(&catalog));
        for _ in 0..10 {
            assert!(gpu.prepare(&catalog, &device, &queue).is_err());
            assert_eq!(gpu.buckets.len(), 1);
        }
    }
}
