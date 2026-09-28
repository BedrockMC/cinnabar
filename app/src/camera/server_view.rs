//! Presentation-only application of server camera instructions, fades, FOV overrides and shakes.
//! Preset registries are not decoded, so `set` applies only explicit pose fields.

use bevy::prelude::{EulerRot, Quat, Resource, Transform, Vec3};
use protocol::{
    CameraEvent, CameraFadeInstruction, CameraFovInstruction, CameraInstructionEvent,
    CameraSetInstruction, CameraShakeAction, CameraShakeEvent, CameraShakeType,
};

use super::{
    easing::{ease, kind_from_name},
    shake::{ShakeKind, ShakeOffset, ShakeState},
};

const MAX_EASE_SECONDS: f32 = 3600.0;

#[derive(Debug, Clone, Copy, PartialEq)]
struct Pose {
    translation: Vec3,
    rotation: Quat,
}

impl Pose {
    fn from_transform(transform: &Transform) -> Self {
        Self {
            translation: transform.translation,
            rotation: transform.rotation,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct PoseBlend {
    from: Pose,
    to: Pose,
    elapsed: f32,
    duration: f32,
    kind: u8,
}

impl PoseBlend {
    fn current(&self) -> Pose {
        let progress = if self.duration > 0.0 {
            ease(self.kind, self.elapsed / self.duration)
        } else {
            1.0
        };
        Pose {
            translation: self.from.translation.lerp(self.to.translation, progress),
            rotation: self
                .from
                .rotation
                .slerp(self.to.rotation, progress)
                .normalize(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct FovBlend {
    from_degrees: f32,
    to_degrees: f32,
    returning: bool,
    elapsed: f32,
    duration: f32,
    kind: u8,
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct FadeState {
    fade_in: f32,
    hold: f32,
    fade_out: f32,
    color: [f32; 3],
    elapsed: f32,
}

impl FadeState {
    fn total(&self) -> f32 {
        self.fade_in + self.hold + self.fade_out
    }

    fn alpha(&self) -> f32 {
        let t = self.elapsed;
        let alpha = if t < self.fade_in {
            t / self.fade_in
        } else if t < self.fade_in + self.hold {
            1.0
        } else if self.fade_out > 0.0 {
            1.0 - (t - self.fade_in - self.hold) / self.fade_out
        } else {
            0.0
        };
        alpha.clamp(0.0, 1.0)
    }
}

/// Counters for well-formed instructions this client cannot apply yet.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ServerCameraSkips {
    pub unresolved_presets: u64,
    pub actor_bound: u64,
    pub unknown_shake: u64,
    pub legacy_switch: u64,
}

/// Live server-driven camera state.
#[derive(Resource, Debug, Default, Clone, Copy, PartialEq)]
pub struct ServerCameraView {
    pose: Option<PoseBlend>,
    fov: Option<FovBlend>,
    fade: Option<FadeState>,
    shake: ShakeState,
    skips: ServerCameraSkips,
    last_sequence: u64,
    seen_resets: u64,
}

impl ServerCameraView {
    #[must_use]
    pub const fn skips(&self) -> ServerCameraSkips {
        self.skips
    }

    #[must_use]
    pub const fn last_sequence(&self) -> u64 {
        self.last_sequence
    }

    /// Restarts sequence tracking and drops state when the retained queue was reset upstream.
    pub fn observe_resets(&mut self, resets: u64) {
        if resets != self.seen_resets {
            self.seen_resets = resets;
            self.clear();
        }
    }

    pub fn clear(&mut self) {
        *self = Self {
            skips: self.skips,
            seen_resets: self.seen_resets,
            ..Self::default()
        };
    }

    #[must_use]
    pub fn shake_offset(&self) -> ShakeOffset {
        self.shake.offset()
    }

    #[must_use]
    pub fn has_pose_override(&self) -> bool {
        self.pose.is_some()
    }

    #[must_use]
    pub fn is_active(&self) -> bool {
        self.pose.is_some() || self.shake.is_active()
    }

    /// Server-authored camera pose, if any.
    #[must_use]
    pub fn pose_override(&self) -> Option<Transform> {
        self.pose.map(|blend| {
            let pose = blend.current();
            Transform {
                translation: pose.translation,
                rotation: pose.rotation,
                ..Transform::IDENTITY
            }
        })
    }

    /// Fade overlay as `(rgb, alpha)` while a fade is running.
    #[must_use]
    pub fn fade_overlay(&self) -> Option<([f32; 3], f32)> {
        self.fade.map(|fade| (fade.color, fade.alpha()))
    }

    /// FOV override in the same degrees as the FOV setting, blended toward `base_degrees` on release.
    #[must_use]
    pub fn fov_override_degrees(&self, base_degrees: f32) -> Option<f32> {
        let blend = self.fov?;
        let progress = if blend.duration > 0.0 {
            ease(blend.kind, blend.elapsed / blend.duration)
        } else {
            1.0
        };
        let target = if blend.returning {
            base_degrees
        } else {
            blend.to_degrees
        };
        Some(blend.from_degrees + (target - blend.from_degrees) * progress)
    }

    pub fn advance(&mut self, delta_seconds: f32) {
        if !(delta_seconds.is_finite() && delta_seconds > 0.0) {
            return;
        }
        if let Some(pose) = &mut self.pose {
            pose.elapsed = (pose.elapsed + delta_seconds).min(pose.duration.max(0.0));
        }
        if let Some(fov) = &mut self.fov {
            fov.elapsed += delta_seconds;
            if fov.returning && fov.elapsed >= fov.duration {
                self.fov = None;
            }
        }
        if let Some(fade) = &mut self.fade {
            fade.elapsed += delta_seconds;
            if fade.elapsed >= fade.total() {
                self.fade = None;
            }
        }
        self.shake.advance(delta_seconds);
    }

    /// Applies one committed event against the camera's current unmodified pose and FOV setting.
    pub fn apply(&mut self, sequence: u64, event: &CameraEvent, base: &Transform, base_fov: f32) {
        self.last_sequence = self.last_sequence.max(sequence);
        match event {
            CameraEvent::Switch(_) => self.skips.legacy_switch += 1,
            CameraEvent::Shake(shake) => self.apply_shake(shake),
            CameraEvent::Instruction(instruction) => {
                self.apply_instruction(instruction, base, base_fov)
            }
        }
    }

    fn apply_shake(&mut self, shake: &CameraShakeEvent) {
        match shake.action {
            CameraShakeAction::Stop => self.shake.stop_all(),
            CameraShakeAction::Add => {
                let kind = match shake.shake_type {
                    CameraShakeType::Positional => ShakeKind::Positional,
                    CameraShakeType::Rotational => ShakeKind::Rotational,
                    CameraShakeType::Unknown(_) => {
                        self.skips.unknown_shake += 1;
                        return;
                    }
                };
                if !self
                    .shake
                    .add(kind, shake.intensity, shake.duration_seconds)
                {
                    self.skips.unknown_shake += 1;
                }
            }
            CameraShakeAction::Unknown(_) => self.skips.unknown_shake += 1,
        }
    }

    fn apply_instruction(
        &mut self,
        instruction: &CameraInstructionEvent,
        base: &Transform,
        base_fov: f32,
    ) {
        if instruction.clear == Some(true) {
            self.pose = None;
        }
        if let Some(set) = &instruction.set {
            self.apply_set(set, base);
        }
        if let Some(fade) = &instruction.fade {
            self.apply_fade(fade);
        }
        if let Some(fov) = &instruction.fov {
            self.apply_fov(fov, base_fov);
        }
        if instruction.target.is_some() || instruction.attach_to_entity.is_some() {
            self.skips.actor_bound += 1;
        }
    }

    fn current_pose(&self, base: &Transform) -> Pose {
        self.pose
            .map_or_else(|| Pose::from_transform(base), |blend| blend.current())
    }

    fn apply_set(&mut self, set: &CameraSetInstruction, base: &Transform) {
        if set.position.is_none() && set.rotation_degrees.is_none() && set.facing_position.is_none()
        {
            self.skips.unresolved_presets += 1;
            return;
        }
        let from = self.current_pose(base);
        let translation = set.position.map_or(from.translation, Vec3::from_array);
        let rotation = match (set.facing_position, set.rotation_degrees) {
            (Some(target), _) => {
                look_rotation(translation, Vec3::from_array(target)).unwrap_or(from.rotation)
            }
            (None, Some([pitch, yaw])) => bedrock_rotation(yaw, pitch),
            (None, None) => from.rotation,
        };
        let (kind, duration) = set.ease.map_or((0, 0.0), |ease| {
            (ease.kind, ease.time_seconds.clamp(0.0, MAX_EASE_SECONDS))
        });
        self.pose = Some(PoseBlend {
            from,
            to: Pose {
                translation,
                rotation,
            },
            elapsed: 0.0,
            duration,
            kind,
        });
    }

    fn apply_fade(&mut self, fade: &CameraFadeInstruction) {
        let previous = self.fade;
        let (fade_in, hold, fade_out) = match (fade.time, previous) {
            (Some(time), _) => (
                time.fade_in_seconds,
                time.hold_seconds,
                time.fade_out_seconds,
            ),
            (None, Some(previous)) => (previous.fade_in, previous.hold, previous.fade_out),
            (None, None) => return,
        };
        let color = fade.color.map_or_else(
            || previous.map_or([0.0; 3], |previous| previous.color),
            |color| [color.red, color.green, color.blue].map(|channel| channel.clamp(0.0, 1.0)),
        );
        let clamp = |seconds: f32| seconds.clamp(0.0, MAX_EASE_SECONDS);
        self.fade = Some(FadeState {
            fade_in: clamp(fade_in),
            hold: clamp(hold),
            fade_out: clamp(fade_out),
            color,
            elapsed: 0.0,
        });
    }

    fn apply_fov(&mut self, fov: &CameraFovInstruction, base_fov: f32) {
        let from = self.fov_override_degrees(base_fov).unwrap_or(base_fov);
        let duration = if fov.ease_time_seconds.is_finite() {
            fov.ease_time_seconds.clamp(0.0, MAX_EASE_SECONDS)
        } else {
            0.0
        };
        let returning = fov.clear || !fov.degrees.is_finite();
        if returning && duration <= 0.0 {
            self.fov = None;
            return;
        }
        self.fov = Some(FovBlend {
            from_degrees: from,
            to_degrees: if returning { base_fov } else { fov.degrees },
            returning,
            elapsed: 0.0,
            duration,
            kind: kind_from_name(&fov.ease_type),
        });
    }
}

fn bedrock_rotation(yaw_degrees: f32, pitch_degrees: f32) -> Quat {
    Quat::from_euler(
        EulerRot::YXZ,
        (180.0 - yaw_degrees).to_radians(),
        -pitch_degrees.to_radians(),
        0.0,
    )
}

fn look_rotation(from: Vec3, target: Vec3) -> Option<Quat> {
    (from.distance_squared(target) > f32::EPSILON).then(|| {
        Transform::from_translation(from)
            .looking_at(target, Vec3::Y)
            .rotation
    })
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use protocol::{CameraEase, CameraFadeColor, CameraFadeTimes};

    use super::*;

    fn set_event(set: CameraSetInstruction) -> CameraEvent {
        CameraEvent::Instruction(CameraInstructionEvent {
            set: Some(set),
            ..Default::default()
        })
    }

    fn empty_set() -> CameraSetInstruction {
        CameraSetInstruction {
            preset_id: 0,
            ease: None,
            position: None,
            rotation_degrees: None,
            facing_position: None,
            view_offset: None,
            entity_offset: None,
            default_preset: None,
            remove_ignore_starting_values: false,
        }
    }

    #[test]
    fn set_with_ease_blends_position_from_the_base_pose() {
        let mut view = ServerCameraView::default();
        let base = Transform::from_xyz(0.0, 0.0, 0.0);
        let mut set = empty_set();
        set.position = Some([10.0, 0.0, 0.0]);
        set.ease = Some(CameraEase {
            kind: 0,
            time_seconds: 2.0,
        });
        view.apply(1, &set_event(set), &base, 90.0);
        assert_eq!(view.pose_override().unwrap().translation, Vec3::ZERO);
        view.advance(1.0);
        assert!((view.pose_override().unwrap().translation.x - 5.0).abs() < 1e-4);
        view.advance(5.0);
        assert!((view.pose_override().unwrap().translation.x - 10.0).abs() < 1e-4);
    }

    #[test]
    fn instant_set_and_clear() {
        let mut view = ServerCameraView::default();
        let mut set = empty_set();
        set.position = Some([1.0, 2.0, 3.0]);
        view.apply(1, &set_event(set), &Transform::IDENTITY, 90.0);
        assert_eq!(
            view.pose_override().unwrap().translation,
            Vec3::new(1.0, 2.0, 3.0)
        );
        view.apply(
            2,
            &CameraEvent::Instruction(CameraInstructionEvent {
                clear: Some(true),
                ..Default::default()
            }),
            &Transform::IDENTITY,
            90.0,
        );
        assert!(view.pose_override().is_none());
        assert_eq!(view.last_sequence(), 2);
    }

    #[test]
    fn bedrock_rotation_zero_yaw_faces_positive_z() {
        let forward = bedrock_rotation(0.0, 0.0) * Vec3::NEG_Z;
        assert!((forward - Vec3::Z).length() < 1e-5);
        let down = bedrock_rotation(0.0, 90.0) * Vec3::NEG_Z;
        assert!(down.y < -0.99);
    }

    #[test]
    fn facing_position_overrides_rotation() {
        let mut view = ServerCameraView::default();
        let mut set = empty_set();
        set.position = Some([0.0, 0.0, 0.0]);
        set.rotation_degrees = Some([0.0, 0.0]);
        set.facing_position = Some([10.0, 0.0, 0.0]);
        view.apply(1, &set_event(set), &Transform::IDENTITY, 90.0);
        let forward = view.pose_override().unwrap().rotation * Vec3::NEG_Z;
        assert!((forward - Vec3::X).length() < 1e-5);
    }

    #[test]
    fn preset_only_set_is_counted_not_applied() {
        let mut view = ServerCameraView::default();
        view.apply(1, &set_event(empty_set()), &Transform::IDENTITY, 90.0);
        assert!(!view.has_pose_override());
        assert_eq!(view.skips().unresolved_presets, 1);
    }

    #[test]
    fn fade_runs_in_hold_out_then_ends() {
        let mut view = ServerCameraView::default();
        view.apply(
            1,
            &CameraEvent::Instruction(CameraInstructionEvent {
                fade: Some(CameraFadeInstruction {
                    time: Some(CameraFadeTimes {
                        fade_in_seconds: 1.0,
                        hold_seconds: 1.0,
                        fade_out_seconds: 1.0,
                    }),
                    color: Some(CameraFadeColor {
                        red: 1.0,
                        green: 0.0,
                        blue: 2.0,
                    }),
                }),
                ..Default::default()
            }),
            &Transform::IDENTITY,
            90.0,
        );
        assert_eq!(view.fade_overlay(), Some(([1.0, 0.0, 1.0], 0.0)));
        view.advance(0.5);
        assert!((view.fade_overlay().unwrap().1 - 0.5).abs() < 1e-5);
        view.advance(1.0);
        assert_eq!(view.fade_overlay().unwrap().1, 1.0);
        view.advance(1.0);
        assert!((view.fade_overlay().unwrap().1 - 0.5).abs() < 1e-5);
        view.advance(1.0);
        assert!(view.fade_overlay().is_none());
    }

    #[test]
    fn fov_override_blends_and_releases_to_the_setting() {
        let mut view = ServerCameraView::default();
        let fov = |degrees: f32, clear: bool| {
            CameraEvent::Instruction(CameraInstructionEvent {
                fov: Some(CameraFovInstruction {
                    degrees,
                    ease_time_seconds: 1.0,
                    ease_type: Arc::from("linear"),
                    clear,
                }),
                ..Default::default()
            })
        };
        view.apply(1, &fov(50.0, false), &Transform::IDENTITY, 90.0);
        assert_eq!(view.fov_override_degrees(90.0), Some(90.0));
        view.advance(0.5);
        assert!((view.fov_override_degrees(90.0).unwrap() - 70.0).abs() < 1e-4);
        view.advance(1.0);
        assert!((view.fov_override_degrees(90.0).unwrap() - 50.0).abs() < 1e-4);
        view.apply(2, &fov(0.0, true), &Transform::IDENTITY, 90.0);
        view.advance(2.0);
        assert_eq!(view.fov_override_degrees(90.0), None);
    }

    #[test]
    fn shakes_route_by_kind_and_stop() {
        let mut view = ServerCameraView::default();
        let shake = |shake_type, action| {
            CameraEvent::Shake(CameraShakeEvent {
                intensity: 1.0,
                duration_seconds: 2.0,
                shake_type,
                action,
            })
        };
        view.apply(
            1,
            &shake(CameraShakeType::Positional, CameraShakeAction::Add),
            &Transform::IDENTITY,
            90.0,
        );
        assert!(view.is_active());
        view.apply(
            2,
            &shake(CameraShakeType::Unknown(9), CameraShakeAction::Add),
            &Transform::IDENTITY,
            90.0,
        );
        assert_eq!(view.skips().unknown_shake, 1);
        view.apply(
            3,
            &shake(CameraShakeType::Positional, CameraShakeAction::Stop),
            &Transform::IDENTITY,
            90.0,
        );
        assert!(!view.is_active());
    }

    #[test]
    fn upstream_reset_drops_state_but_keeps_counters() {
        let mut view = ServerCameraView::default();
        let mut set = empty_set();
        set.position = Some([1.0, 0.0, 0.0]);
        view.apply(1, &set_event(empty_set()), &Transform::IDENTITY, 90.0);
        view.apply(2, &set_event(set), &Transform::IDENTITY, 90.0);
        view.observe_resets(1);
        assert!(!view.has_pose_override());
        assert_eq!(view.skips().unresolved_presets, 1);
    }
}
