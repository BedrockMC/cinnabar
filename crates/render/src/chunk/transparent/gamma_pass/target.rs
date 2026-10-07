use crate::chunk::*;
use crate::scene_sampling::SceneCopy;

/// One scene-sized scratch texture per admitted camera, with two views of the
/// same encoded bytes. Resize replaces it; no unbounded historical target map.
#[derive(Component)]
pub(super) struct GammaTarget {
    pub(super) texture: Texture,
    pub(super) gamma_view: TextureView,
    pub(super) srgb_view: TextureView,
    multisample: Option<(Texture, TextureView, TextureView)>,
    pub(super) copy: Option<SceneCopy>,
}

impl GammaTarget {
    /// Both colour-space views refer to the same per-sample colour storage.
    pub(super) fn colour_view(&self, gamma: bool) -> &TextureView {
        match &self.multisample {
            Some((_, encoded, linear)) => {
                if gamma {
                    encoded
                } else {
                    linear
                }
            }
            None => {
                if gamma {
                    &self.gamma_view
                } else {
                    &self.srgb_view
                }
            }
        }
    }

    /// Each sorted range resolves into the corresponding view of the shared encoded image.
    pub(super) fn resolve_view(&self, gamma: bool) -> Option<&TextureView> {
        self.multisample.as_ref().map(|_| {
            if gamma {
                &self.gamma_view
            } else {
                &self.srgb_view
            }
        })
    }
}

use super::admitted;

type GammaTargetViews<'w, 's> = Query<
    'w,
    's,
    (
        Entity,
        &'static ExtractedView,
        &'static ViewTarget,
        &'static Msaa,
        Option<&'static crate::EnhancedRendering>,
        Option<&'static GammaTarget>,
    ),
    With<Camera3d>,
>;

pub(super) fn prepare_gamma_targets(
    mut commands: Commands,
    device: Res<RenderDevice>,
    views: GammaTargetViews,
) {
    for (entity, view, target, msaa, enhanced, previous) in &views {
        if !admitted(view.hdr, *msaa, enhanced.is_some()) {
            if previous.is_some() {
                commands.entity(entity).remove::<GammaTarget>();
            }
            continue;
        }
        let size = target.main_texture().size();
        if previous.is_some_and(|scratch| {
            scratch.texture.size() == size
                && scratch
                    .multisample
                    .as_ref()
                    .map_or(1, |(texture, _, _)| texture.sample_count())
                    == msaa.samples()
                && scratch
                    .copy
                    .as_ref()
                    .is_none_or(|copy| copy.matches(target, msaa.samples()))
        }) {
            continue;
        }
        let srgb = TextureFormat::bevy_default();
        let gamma = srgb.remove_srgb_suffix();
        let texture = device.create_texture(&TextureDescriptor {
            label: Some("ordinary gamma transparent scene"),
            size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: TextureDimension::D2,
            format: gamma,
            usage: TextureUsages::RENDER_ATTACHMENT
                | TextureUsages::COPY_SRC
                | TextureUsages::COPY_DST,
            view_formats: &[srgb],
        });
        let gamma_view = texture.create_view(&TextureViewDescriptor::default());
        let srgb_view = texture.create_view(&TextureViewDescriptor {
            format: Some(srgb),
            ..default()
        });
        let multisample = (msaa.samples() > 1).then(|| {
            let texture = device.create_texture(&TextureDescriptor {
                label: Some("ordinary gamma multisample scene"),
                size,
                mip_level_count: 1,
                sample_count: msaa.samples(),
                dimension: TextureDimension::D2,
                format: gamma,
                usage: TextureUsages::RENDER_ATTACHMENT,
                view_formats: &[srgb],
            });
            let encoded = texture.create_view(&TextureViewDescriptor::default());
            let linear = texture.create_view(&TextureViewDescriptor {
                format: Some(srgb),
                ..default()
            });
            (texture, encoded, linear)
        });
        commands.entity(entity).insert(GammaTarget {
            texture,
            gamma_view,
            srgb_view,
            multisample,
            copy: (msaa.samples() > 1).then(|| {
                SceneCopy::new(
                    &device,
                    target,
                    msaa.samples(),
                    crate::RuntimeStage::GpuTransparent,
                )
            }),
        });
    }
}
