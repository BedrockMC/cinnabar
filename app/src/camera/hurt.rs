//! Hurt camera tilt toward the damage direction; presentation-only, never feeds look or movement.
//! Duration, peak angle, curve and sign need native measurement.

use std::f32::consts::PI;

use bevy::prelude::{Mat4, Resource, Vec3};

const HURT_DURATION_SECONDS: f32 = 0.5;
const HURT_TILT_DEGREES: f32 = 14.0;

/// One committed local-player damage event.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LocalHurtEvent {
    /// World-space horizontal `(x, z)` vector from the player toward the damage source.
    pub source_direction: Option<[f32; 2]>,
}

#[derive(Resource, Debug, Default, Clone, Copy, PartialEq)]
pub struct CameraHurtState {
    remaining_seconds: f32,
    source_direction: Option<[f32; 2]>,
}

impl CameraHurtState {
    /// Restarts the tilt; a non-finite direction is treated as directionless.
    pub fn register(&mut self, event: LocalHurtEvent) {
        self.remaining_seconds = HURT_DURATION_SECONDS;
        self.source_direction = event
            .source_direction
            .filter(|direction| direction.iter().all(|value| value.is_finite()))
            .filter(|direction| direction[0].hypot(direction[1]) > f32::EPSILON);
    }

    pub fn advance(&mut self, delta_seconds: f32) {
        if delta_seconds.is_finite() && delta_seconds > 0.0 {
            self.remaining_seconds = (self.remaining_seconds - delta_seconds).max(0.0);
        }
    }

    pub fn clear(&mut self) {
        *self = Self::default();
    }

    #[must_use]
    pub fn is_active(&self) -> bool {
        self.remaining_seconds > 0.0
    }

    /// View-space rotation for the current tilt at the given view yaw (radians).
    #[must_use]
    pub fn view_matrix(&self, view_yaw: f32) -> Mat4 {
        if !self.is_active() {
            return Mat4::IDENTITY;
        }
        let progress = (self.remaining_seconds / HURT_DURATION_SECONDS).clamp(0.0, 1.0);
        let tilt = (progress.powi(4) * PI).sin() * HURT_TILT_DEGREES.to_radians();
        let relative = self.source_direction.map_or(PI * 0.5, |[x, z]| {
            let (sin, cos) = view_yaw.sin_cos();
            // Bevy forward is (-sin, -cos) and right is (cos, -sin) at yaw `view_yaw`.
            let forward = -sin * x - cos * z;
            let right = cos * x - sin * z;
            right.atan2(forward)
        });
        let axis = Vec3::new(relative.cos(), 0.0, -relative.sin());
        Mat4::from_axis_angle(axis, -tilt)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn active() -> CameraHurtState {
        let mut state = CameraHurtState::default();
        state.register(LocalHurtEvent {
            source_direction: Some([0.0, -1.0]),
        });
        state
    }

    #[test]
    fn inactive_is_identity_and_expires() {
        assert_eq!(CameraHurtState::default().view_matrix(0.0), Mat4::IDENTITY);
        let mut state = active();
        state.advance(0.6);
        assert!(!state.is_active());
        assert_eq!(state.view_matrix(0.0), Mat4::IDENTITY);
    }

    #[test]
    fn tilt_peaks_mid_decay_and_starts_and_ends_at_rest() {
        let mut state = active();
        let start = state.view_matrix(0.0);
        assert!(start.abs_diff_eq(Mat4::IDENTITY, 1e-5));
        state.advance(0.08);
        assert!(!state.view_matrix(0.0).abs_diff_eq(Mat4::IDENTITY, 1e-3));
        state.advance(0.41);
        assert!(state.view_matrix(0.0).abs_diff_eq(Mat4::IDENTITY, 1e-3));
    }

    #[test]
    fn front_source_pitches_and_side_source_rolls() {
        // Yaw 0 looks toward -Z, so (0,-1) is dead ahead and (1,0) is to the right.
        let mut ahead = CameraHurtState::default();
        ahead.register(LocalHurtEvent {
            source_direction: Some([0.0, -1.0]),
        });
        let mut side = CameraHurtState::default();
        side.register(LocalHurtEvent {
            source_direction: Some([1.0, 0.0]),
        });
        ahead.advance(0.08);
        side.advance(0.08);
        let a = ahead.view_matrix(0.0);
        let s = side.view_matrix(0.0);
        assert!(a.y_axis.z.abs() > 0.05 && a.x_axis.y.abs() < 1e-4);
        assert!(s.x_axis.y.abs() > 0.05 && s.y_axis.z.abs() < 1e-4);
    }

    #[test]
    fn malformed_direction_degrades_to_directionless() {
        let mut state = CameraHurtState::default();
        state.register(LocalHurtEvent {
            source_direction: Some([f32::NAN, 1.0]),
        });
        state.advance(0.08);
        assert!(state.view_matrix(0.0).is_finite());
        assert!(state.is_active());
    }
}
