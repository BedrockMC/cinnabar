use super::*;
use bevy::render::render_resource::DepthBiasState;

pub(super) const NATIVE_ENVIRONMENTAL_TEXT_DEPTH_BIAS: i32 =
    -crate::nametag::NAMETAG_TEXT_REVERSE_Z_BIAS;

#[derive(Clone, Copy, PartialEq, Eq, Hash, SpecializerKey)]
pub(super) struct UiPipelineKey {
    pub(super) msaa: Msaa,
    pub(super) hdr: bool,
    pub(super) invert_blend: bool,
    pub(super) layer: bool,
    pub(super) depth_test: bool,
    pub(super) depth_write: bool,
}

impl Specializer<RenderPipeline> for UiPipelineSpecializer {
    type Key = UiPipelineKey;

    fn specialize(
        &self,
        key: Self::Key,
        descriptor: &mut RenderPipelineDescriptor,
    ) -> Result<Canonical<Self::Key>, BevyError> {
        descriptor.multisample.count = if key.layer { 1 } else { key.msaa.samples() };
        descriptor.fragment.as_mut().unwrap().entry_point = Some(
            if !key.layer && !key.invert_blend {
                "ui_world_fragment"
            } else {
                "ui_fragment"
            }
            .into(),
        );
        descriptor.depth_stencil =
            (key.depth_test || key.depth_write).then_some(DepthStencilState {
                format: CORE_3D_DEPTH_FORMAT,
                depth_write_enabled: key.depth_write,
                depth_compare: if key.depth_test {
                    CompareFunction::GreaterEqual
                } else {
                    CompareFunction::Always
                },
                stencil: default(),
                // Depth-tested, depth-writing projected UI is the native environmental-text
                // mode. Plates are read-only; ordinary text uses Always; HUD has no depth state.
                bias: DepthBiasState {
                    constant: if key.depth_test && key.depth_write {
                        -NATIVE_ENVIRONMENTAL_TEXT_DEPTH_BIAS
                    } else {
                        0
                    },
                    ..default()
                },
            });
        let target = descriptor.fragment.as_mut().unwrap().targets[0]
            .as_mut()
            .unwrap();
        target.format = if key.layer {
            composite::UI_LAYER_FORMAT
        } else if key.hdr {
            ViewTarget::TEXTURE_FORMAT_HDR
        } else {
            TextureFormat::bevy_default()
        };
        target.blend = Some(if key.invert_blend {
            ui_invert_blend_state()
        } else {
            ui_alpha_blend_state()
        });
        Ok(key)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hud_layer_and_projected_world_use_distinct_color_targets() {
        for layer in [true, false] {
            let mut descriptor = ui_pipeline_descriptor(ui_bind_group_layout());
            UiPipelineSpecializer
                .specialize(
                    UiPipelineKey {
                        msaa: Msaa::Sample4,
                        hdr: true,
                        invert_blend: false,
                        layer,
                        depth_test: !layer,
                        depth_write: !layer,
                    },
                    &mut descriptor,
                )
                .unwrap();
            let fragment = descriptor.fragment.unwrap();
            assert_eq!(
                fragment.entry_point.as_deref(),
                Some(if layer {
                    "ui_fragment"
                } else {
                    "ui_world_fragment"
                })
            );
            assert_eq!(
                fragment.targets[0].as_ref().unwrap().format,
                if layer {
                    composite::UI_LAYER_FORMAT
                } else {
                    ViewTarget::TEXTURE_FORMAT_HDR
                }
            );
            assert_eq!(
                descriptor.multisample.count,
                if layer { 1 } else { Msaa::Sample4.samples() }
            );
            assert_eq!(descriptor.depth_stencil.is_some(), !layer);
        }
    }
}
