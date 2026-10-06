//! Java Edition 1.7 view bob, hand sway and hurt roll, advanced on physics ticks.
//! Rules: docs/reference/java-1-7-animations.md.

use bevy::math::DVec3;
use bevy::prelude::{Mat4, Resource, Vec3};
use render_model::java_animation::{java_cos, java_sin};

use super::bob::{ViewEffect, shortest_degrees};

/// Previous and current tick values of Java's camera bob and hand sway inputs.
#[derive(Resource, Debug, Default, Clone, Copy, PartialEq)]
pub struct JavaCameraState {
    last: Option<(u64, DVec3)>,
    walked: [f32; 2],
    bob: [f32; 2],
    tilt: [f32; 2],
    arm_pitch: [f32; 2],
    arm_yaw: [f32; 2],
}

/// One completed physics tick as Java's bob and sway read it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct JavaCameraTick {
    pub tick: u64,
    pub position: DVec3,
    pub velocity: DVec3,
    pub on_ground: bool,
    pub alive: bool,
    pub riding: bool,
    /// Java stops the walk phase while flying, riding or sneaking on the ground.
    pub walks: bool,
    /// Look pitch and yaw in Minecraft degrees (down and right positive).
    pub look: [f32; 2],
}

const WALK_PER_BLOCK: f64 = 0.6;
const BOB_CAP: f32 = 0.1;
const BOB_FOLLOW: f32 = 0.4;
const TILT_FOLLOW: f32 = 0.8;
const ARM_FOLLOW: f32 = 0.5;
const SWAY: f32 = 0.1;
const TELEPORT_BLOCKS: f64 = 8.0;
const MAX_CATCH_UP_TICKS: u64 = 20;
const HURT_TILT_DEGREES: f32 = 14.0;

impl JavaCameraState {
    /// Steps once per newly completed tick; a missed run of ticks shares the movement.
    pub fn advance(&mut self, tick: JavaCameraTick) {
        let Some((last_tick, last_position)) = self.last else {
            self.last = Some((tick.tick, tick.position));
            self.arm_pitch = [tick.look[0]; 2];
            self.arm_yaw = [tick.look[1]; 2];
            return;
        };
        if tick.tick == last_tick {
            return;
        }
        self.last = Some((tick.tick, tick.position));
        let delta = tick.position - last_position;
        if tick.tick < last_tick || delta.length() > TELEPORT_BLOCKS {
            *self = Self {
                last: self.last,
                ..Self::default()
            };
            self.arm_pitch = [tick.look[0]; 2];
            self.arm_yaw = [tick.look[1]; 2];
            return;
        }
        let steps = (tick.tick - last_tick).min(MAX_CATCH_UP_TICKS);
        let step = delta / steps as f64;
        for _ in 0..steps {
            self.step(&tick, step);
        }
    }

    fn step(&mut self, tick: &JavaCameraTick, delta: DVec3) {
        for pair in [
            &mut self.walked,
            &mut self.bob,
            &mut self.tilt,
            &mut self.arm_pitch,
            &mut self.arm_yaw,
        ] {
            pair[0] = pair[1];
        }
        if tick.walks {
            let distance = (delta.x * delta.x + delta.z * delta.z).sqrt() as f32;
            if distance.is_finite() {
                self.walked[1] =
                    (f64::from(self.walked[1]) + f64::from(distance) * WALK_PER_BLOCK) as f32;
            }
        }
        let speed =
            (tick.velocity.x * tick.velocity.x + tick.velocity.z * tick.velocity.z).sqrt() as f32;
        let speed = if tick.on_ground && tick.alive && !tick.riding && speed.is_finite() {
            speed.min(BOB_CAP)
        } else {
            0.0
        };
        let tilt = if tick.on_ground || !tick.alive || tick.riding || !tick.velocity.y.is_finite() {
            0.0
        } else {
            (-tick.velocity.y * f64::from(0.2_f32)).atan() as f32 * 15.0
        };
        self.bob[1] = if tick.riding {
            0.0
        } else {
            self.bob[1] + (speed - self.bob[1]) * BOB_FOLLOW
        };
        self.tilt[1] += (tilt - self.tilt[1]) * TILT_FOLLOW;
        self.arm_pitch[1] += (tick.look[0] - self.arm_pitch[1]) * ARM_FOLLOW;
        self.arm_yaw[1] += shortest_degrees(tick.look[1] - self.arm_yaw[1]) * ARM_FOLLOW;
    }

    /// The bob at `alpha`; Java runs the walk phase one tick ahead of the frame.
    #[must_use]
    pub fn bob(&self, alpha: f32) -> ViewEffect {
        let [previous, current] = self.walked;
        let phase = -(current + (current - previous) * alpha) * std::f32::consts::PI;
        let bob = lerp(self.bob, alpha);
        let tilt = lerp(self.tilt, alpha);
        ViewEffect {
            translation: Vec3::new(
                java_sin(phase) * bob * 0.5,
                -(java_cos(phase) * bob).abs(),
                0.0,
            ),
            roll_radians: (java_sin(phase) * bob * 3.0).to_radians(),
            pitch_radians: ((java_cos(phase - 0.2) * bob).abs() * 5.0 + tilt).to_radians(),
        }
    }

    /// Hand `(pitch, yaw)` sway in radians: the live look ahead of the lagging arm angles.
    #[must_use]
    pub fn sway(&self, alpha: f32, look: [f32; 2]) -> (f32, f32) {
        let pitch = (look[0] - lerp(self.arm_pitch, alpha)) * SWAY;
        let arm_yaw = self.arm_yaw[0] + shortest_degrees(self.arm_yaw[1] - self.arm_yaw[0]) * alpha;
        let yaw = shortest_degrees(look[1] - arm_yaw) * SWAY;
        (pitch.to_radians(), yaw.to_radians())
    }
}

/// Java's hurt roll for `progress` (hurt time left over its duration); the client never
/// learns a direction, so it only rolls.
#[must_use]
pub fn java_hurt_roll(progress: f32) -> Mat4 {
    if progress <= 0.0 {
        return Mat4::IDENTITY;
    }
    let shake = java_sin(progress.powi(4) * std::f32::consts::PI);
    Mat4::from_rotation_z((-shake * HURT_TILT_DEGREES).to_radians())
}

fn lerp([previous, current]: [f32; 2], alpha: f32) -> f32 {
    previous + (current - previous) * alpha
}

#[cfg(test)]
mod tests {
    use super::*;

    fn walking(tick: u64, x: f32) -> JavaCameraTick {
        JavaCameraTick {
            tick,
            position: DVec3::new(f64::from(x), 64.0, 0.0),
            velocity: DVec3::new(0.2, -0.0784, 0.0),
            on_ground: true,
            alive: true,
            riding: false,
            walks: true,
            look: [0.0, 0.0],
        }
    }

    /// Walk phase gains 0.6 per block and the bob eases 40% toward the capped speed.
    #[test]
    fn bob_follows_java_tick_rules() {
        let mut state = JavaCameraState::default();
        state.advance(walking(1, 0.0));
        state.advance(walking(2, 0.2));
        assert!((state.walked[1] - 0.12).abs() < 1e-6);
        assert!((state.bob[1] - 0.04).abs() < 1e-6);
        state.advance(walking(3, 0.4));
        assert!((state.bob[1] - (0.04 + 0.06 * 0.4)).abs() < 1e-6);
        let effect = state.bob(0.5);
        let phase = -(0.24 + 0.12 * 0.5) * std::f32::consts::PI;
        let bob = 0.04 + 0.024 * 0.5;
        assert!((effect.translation.x - phase.sin() * bob * 0.5).abs() < 1e-4);
        assert!((effect.translation.y + (phase.cos() * bob).abs()).abs() < 1e-4);
    }

    /// Airborne the bob decays and the fall tilt eases in at 80% a tick.
    #[test]
    fn falling_tilts_and_freezes_the_bob() {
        let mut state = JavaCameraState::default();
        state.advance(walking(1, 0.0));
        state.advance(JavaCameraTick {
            on_ground: false,
            velocity: DVec3::new(0.0, -0.5, 0.0),
            ..walking(2, 0.0)
        });
        let expected = (0.1_f64.atan() * 15.0 * 0.8) as f32;
        assert!((state.tilt[1] - expected).abs() < 1e-5);
        assert_eq!(state.bob[1], 0.0);
    }

    #[test]
    fn dead_camera_bob_and_fall_tilt_decay_to_zero() {
        let mut state = JavaCameraState {
            bob: [0.04; 2],
            tilt: [6.0; 2],
            ..Default::default()
        };
        state.step(
            &JavaCameraTick {
                alive: false,
                ..walking(2, 0.0)
            },
            DVec3::ZERO,
        );
        assert!((state.bob[1] - 0.024).abs() < 1e-8);
        assert!((state.tilt[1] - 1.2).abs() < 1e-6);
        state.step(
            &JavaCameraTick {
                alive: false,
                on_ground: false,
                velocity: DVec3::new(0.2, -0.5, 0.0),
                ..walking(3, 0.0)
            },
            DVec3::ZERO,
        );
        assert!((state.tilt[1] - 0.24).abs() < 1e-6);
    }

    #[test]
    fn mounted_camera_clears_bob_and_does_not_add_vehicle_fall_tilt() {
        let mut state = JavaCameraState {
            bob: [0.04; 2],
            tilt: [6.0; 2],
            walked: [0.5; 2],
            ..Default::default()
        };
        state.step(
            &JavaCameraTick {
                riding: true,
                walks: false,
                on_ground: false,
                velocity: DVec3::new(0.2, -0.5, 0.0),
                ..walking(2, 0.2)
            },
            DVec3::new(0.2, -0.5, 0.0),
        );
        assert_eq!(state.bob, [0.04, 0.0]);
        assert!((state.tilt[1] - 1.2).abs() < 1e-6);
        assert_eq!(state.walked, [0.5; 2]);
    }

    #[test]
    fn camera_walk_distance_rounds_after_the_double_precision_increment() {
        let mut state = JavaCameraState {
            walked: [0.1002; 2],
            ..Default::default()
        };
        state.step(
            &walking(2, 0.0),
            Vec3::new(-0.034_368_105, 0.0, 0.074_893_28).as_dvec3(),
        );
        assert_eq!(state.walked[1].to_bits(), 0x3e19_3b9e);
    }

    #[test]
    fn camera_fall_tilt_keeps_native_velocity_and_float_cast_order() {
        let mut state = JavaCameraState::default();
        state.step(
            &JavaCameraTick {
                on_ground: false,
                velocity: DVec3::new(0.0, -0.0007, 0.0),
                ..walking(2, 0.0)
            },
            DVec3::ZERO,
        );
        assert_eq!(state.tilt[1].to_bits(), 0x3adc_3373);
    }

    /// The arm angles move halfway to the look each tick; sway is a tenth of the gap.
    #[test]
    fn sway_trails_the_look_by_a_tenth() {
        let mut state = JavaCameraState::default();
        state.advance(walking(1, 0.0));
        state.advance(JavaCameraTick {
            look: [20.0, 350.0],
            ..walking(2, 0.0)
        });
        assert_eq!(state.arm_pitch[1], 10.0);
        assert!((state.arm_yaw[1] + 5.0).abs() < 1e-4);
        let (pitch, yaw) = state.sway(1.0, [20.0, 350.0]);
        assert!((pitch.to_degrees() - 1.0).abs() < 1e-4);
        assert!((yaw.to_degrees() + 0.5).abs() < 1e-4);
    }

    #[test]
    fn hurt_roll_peaks_mid_shake_and_rests_at_the_ends() {
        assert_eq!(java_hurt_roll(0.0), Mat4::IDENTITY);
        let peak = 0.5_f32.powf(0.25);
        let rolled = java_hurt_roll(peak).transform_vector3(Vec3::X);
        assert!((rolled.y + 14.0_f32.to_radians().sin()).abs() < 1e-3);
        assert!(java_hurt_roll(1.0).abs_diff_eq(Mat4::IDENTITY, 1e-3));
    }
}
