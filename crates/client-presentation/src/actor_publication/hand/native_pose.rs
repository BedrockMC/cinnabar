//! Read-only native arm and item-parent poses, retained for unchanged captured frames.
use super::*;
use std::borrow::Cow;

type RenderPose = Arc<[render_model::RenderBoneTransform]>;

#[derive(PartialEq)]
struct PoseKey {
    source: Option<SourceKey>,
    tick: (u32, u64, u64),
    actor_alpha: u32,
    physics_alpha: Option<u32>,
    camera: Option<([f32; 2], [f32; 3])>,
}

/// Keeps one lifetime's sampled pose instead of resampling unchanged hand inputs.
#[derive(Default)]
pub(in super::super) struct NativePoseCache {
    key: Option<PoseKey>,
    pose: Option<[RenderPose; 2]>,
}

impl NativePoseCache {
    /// Samples the complete native parent pose without changing variables or committed clocks.
    pub(super) fn sample(
        &mut self,
        stream: &WorldStream,
        presentation: &ActorRigPresentation,
        consume: Option<u32>,
        animation: Option<client_world::AttachableAnimationInput<'static>>,
        alpha: f32,
        camera: Option<([f32; 2], [f32; 3])>,
    ) -> Option<[RenderPose; 2]> {
        let runtime_id = presentation.submission.input.identity.runtime_id;
        let rig = stream.authority().actor_rig(runtime_id)?;
        let key = PoseKey {
            source: source_key(stream, consume, animation, alpha),
            tick: (rig.rig.0, rig.completed_tick, rig.reset_generation),
            actor_alpha: alpha.to_bits(),
            physics_alpha: rig.java.local_swing_alpha.map(f32::to_bits),
            camera,
        };
        if camera.is_some() && self.key.as_ref() == Some(&key) {
            return self.pose.clone();
        }
        let mut frame = stream.authority().actor_render_frame(alpha);
        let pose = frame.layers(runtime_id).and_then(|layers| {
            let Cow::Owned(layers) = layers else {
                return None;
            };
            let layer = layers.iter().find(|layer| {
                layer.geometry.is_none()
                    && layer.pose.len() == presentation.submission.input.current_bones.len()
                    && layer.previous_pose.len()
                        == presentation.submission.input.previous_bones.len()
                    && !layer.pose.is_empty()
            })?;
            let current = crate::presentation::actors::convert_bones(&layer.pose)?;
            let previous = if Arc::ptr_eq(&layer.previous_pose, &layer.pose) {
                Arc::clone(&current)
            } else {
                crate::presentation::actors::convert_bones(&layer.previous_pose)?
            };
            Some([previous, current])
        });
        self.key = Some(key);
        self.pose = pose.clone();
        pose
    }
}
