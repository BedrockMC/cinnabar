use super::*;
use bevy::render::render_resource::DepthBiasState;

// Matched 1.26.50.26 environmental text, RVA 0x0682c9cf: native constant bias -32.
// The adjacent 0x0682c9d9 override zeros slope/clamp. Native LessEqual uses standard Z;
// our GreaterEqual reverse-Z comparison reverses the bias sign to retain the toward-eye shift.
pub(super) const NATIVE_ENVIRONMENTAL_TEXT_DEPTH_BIAS: i32 =
    -crate::nametag::NAMETAG_TEXT_REVERSE_Z_BIAS;

#[derive(Clone, Copy, PartialEq, Eq, Hash, SpecializerKey)]
pub(super) struct UiPipelineKey {
    pub(super) msaa: Msaa,
    pub(super) hdr: bool,
    pub(super) invert_blend: bool,
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
        descriptor.multisample.count = key.msaa.samples();
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
        target.format = if key.hdr {
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
