//! Non-walking locomotion: ability flight, pose-swimming and elytra gliding.
//!
//! Every coefficient here is a provisional value with no bedsim oracle; each
//! needs independent measurement against a native client before parity is claimed.

use crate::{
    CollisionWorld, Vec3,
    math::{minecraft_cos, minecraft_sin},
};

use super::{
    COLLISION_EPSILON, ControlledTickResult, INPUT_IMPULSE_MULTIPLIER, MovementInput, MovementMode,
    NORMAL_GRAVITY, PlayerState, SimulationError, TickResult, apply_relative_movement,
    collision::resolve_motion, controls, effects, environment::SampledEnvironment,
    scaffolding::ScaffoldingView,
};

const DEFAULT_FLY_SPEED: f64 = 0.05;
const FLY_SPRINT_MULTIPLIER: f64 = 2.0;
const FLY_VERTICAL_MULTIPLIER: f64 = 3.0;
const FLY_VERTICAL_DRAG: f64 = 0.6;
const FLY_HORIZONTAL_DRAG: f64 = 0.91;

const SWIM_ACCELERATION: f64 = 0.02;
const SWIM_DRAG: f64 = 0.9;
const SWIM_WATER_GRAVITY: f64 = 0.005;

const GLIDE_LIFT_SCALE: f64 = 0.75;
const GLIDE_FALL_CONVERSION: f64 = 0.1;
const GLIDE_CLIMB_CONVERSION: f64 = 0.04;
const GLIDE_CLIMB_VERTICAL_BOOST: f64 = 3.2;
const GLIDE_ALIGNMENT: f64 = 0.1;
const GLIDE_DRAG: [f64; 3] = [0.99, 0.98, 0.99];
const SLOW_FALLING_GRAVITY: f64 = 0.01;

pub(super) fn tick_mode(
    mut next: PlayerState,
    state: &mut PlayerState,
    input: MovementInput,
    controls: controls::ProcessedControls,
    sampled: SampledEnvironment,
    grounded_at_start: bool,
    world: &impl CollisionWorld,
) -> Result<ControlledTickResult, SimulationError> {
    let mut controls = controls;
    let in_water = sampled.movement.in_water;
    match input.mode {
        MovementMode::Flying => {
            // Sneak descends while flying instead of slowing the walk.
            controls = controls::process(MovementInput {
                sneaking: false,
                ..input
            });
            let base = input.fly_speed.unwrap_or(DEFAULT_FLY_SPEED);
            let speed = if input.sprinting {
                base * FLY_SPRINT_MULTIPLIER
            } else {
                base
            };
            apply_relative_movement(
                &mut next.velocity,
                controls.move_vector[0] * INPUT_IMPULSE_MULTIPLIER,
                controls.move_vector[1] * INPUT_IMPULSE_MULTIPLIER,
                input.yaw_degrees,
                speed,
            );
            let vertical = f64::from(i8::from(input.jumping) - i8::from(input.sneaking));
            next.velocity.y += vertical * base * FLY_VERTICAL_MULTIPLIER;
        }
        MovementMode::Swimming if in_water => {
            let look = look_vector(input.yaw_degrees, input.pitch_degrees);
            let forward = controls.move_vector[1] * INPUT_IMPULSE_MULTIPLIER * SWIM_ACCELERATION;
            next.velocity += look * forward;
            apply_relative_movement(
                &mut next.velocity,
                controls.move_vector[0] * INPUT_IMPULSE_MULTIPLIER,
                0.0,
                input.yaw_degrees,
                SWIM_ACCELERATION,
            );
        }
        MovementMode::Gliding => {
            next.velocity = glide_velocity(next.velocity, input);
        }
        _ => {}
    }

    let view = ScaffoldingView::new(world, next.position.y, input.sneaking);
    let height = input.mode.hitbox_height(input.sneaking);
    let motion = resolve_motion(
        &view,
        next.position,
        next.velocity,
        grounded_at_start,
        height,
    )?;
    let identity = sampled.identity.merge(&motion.identity)?;
    let pre_collision_velocity = next.velocity;
    next.position += motion.resolved;
    next.on_ground = motion.stepped
        || (motion.collisions.y && pre_collision_velocity.y < 0.0)
        || (grounded_at_start
            && !motion.collisions.y
            && pre_collision_velocity.y.abs() <= COLLISION_EPSILON);
    next.movement = motion.resolved;
    next.velocity = motion.resolved;
    if motion.stepped || motion.collisions.y {
        next.velocity.y = 0.0;
    }
    if motion.collisions.x {
        next.velocity.x = 0.0;
    }
    if motion.collisions.z {
        next.velocity.z = 0.0;
    }

    match input.mode {
        MovementMode::Flying => {
            next.velocity.x *= FLY_HORIZONTAL_DRAG;
            next.velocity.z *= FLY_HORIZONTAL_DRAG;
            next.velocity.y *= FLY_VERTICAL_DRAG;
        }
        MovementMode::Swimming if in_water => {
            next.velocity.x *= SWIM_DRAG;
            next.velocity.y *= SWIM_DRAG;
            next.velocity.z *= SWIM_DRAG;
            effects::apply_vertical(&mut next.velocity.y, input.effects, SWIM_WATER_GRAVITY, 1.0);
        }
        MovementMode::Gliding => {}
        _ => {
            effects::apply_vertical(
                &mut next.velocity.y,
                input.effects,
                NORMAL_GRAVITY,
                super::NORMAL_GRAVITY_MULTIPLIER,
            );
            next.velocity.x *= super::DEFAULT_AIR_FRICTION;
            next.velocity.z *= super::DEFAULT_AIR_FRICTION;
        }
    }
    next.jump_delay = next.jump_delay.saturating_sub(1);
    next.collisions = motion.collisions;

    let result = TickResult {
        tick: next.tick,
        position: next.position,
        velocity: next.velocity,
        movement: next.movement,
        collisions: motion.collisions,
        on_ground: next.on_ground,
        environment: sampled.movement,
        world_identity: identity,
    };
    *state = next;
    Ok(ControlledTickResult {
        tick_result: result,
        controls,
    })
}

/// Unit look direction; pitch is positive downward.
fn look_vector(yaw_degrees: f64, pitch_degrees: f64) -> Vec3 {
    let yaw = yaw_degrees.to_radians();
    let pitch = pitch_degrees.to_radians();
    let horizontal = minecraft_cos(pitch);
    Vec3::new(
        -minecraft_sin(yaw) * horizontal,
        -minecraft_sin(pitch),
        minecraft_cos(yaw) * horizontal,
    )
}

fn glide_velocity(velocity: Vec3, input: MovementInput) -> Vec3 {
    let pitch = input.pitch_degrees.to_radians();
    let look = look_vector(input.yaw_degrees, input.pitch_degrees);
    let look_horizontal = look.x.hypot(look.z);
    let speed_horizontal = velocity.x.hypot(velocity.z);
    let lift = minecraft_cos(pitch) * minecraft_cos(pitch);
    let gravity = if input.effects.slow_falling && velocity.y < 0.0 {
        SLOW_FALLING_GRAVITY
    } else {
        NORMAL_GRAVITY
    };
    let mut next = velocity;
    next.y += gravity * (-1.0 + lift * GLIDE_LIFT_SCALE);
    if look_horizontal > 0.0 {
        if next.y < 0.0 {
            let converted = next.y * -GLIDE_FALL_CONVERSION * lift;
            next.y += converted;
            next.x += look.x * converted / look_horizontal;
            next.z += look.z * converted / look_horizontal;
        }
        if pitch < 0.0 {
            let converted = speed_horizontal * -minecraft_sin(pitch) * GLIDE_CLIMB_CONVERSION;
            next.y += converted * GLIDE_CLIMB_VERTICAL_BOOST;
            next.x -= look.x * converted / look_horizontal;
            next.z -= look.z * converted / look_horizontal;
        }
        next.x += (look.x / look_horizontal * speed_horizontal - next.x) * GLIDE_ALIGNMENT;
        next.z += (look.z / look_horizontal * speed_horizontal - next.z) * GLIDE_ALIGNMENT;
    }
    Vec3::new(
        next.x * GLIDE_DRAG[0],
        next.y * GLIDE_DRAG[1],
        next.z * GLIDE_DRAG[2],
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn glide_input(pitch_degrees: f64) -> MovementInput {
        MovementInput {
            mode: MovementMode::Gliding,
            pitch_degrees,
            ..MovementInput::default()
        }
    }

    #[test]
    fn look_vector_is_unit_and_points_down_for_positive_pitch() {
        let look = look_vector(0.0, 45.0);
        assert!((look.length_squared() - 1.0).abs() < 1.0e-3);
        assert!(look.y < 0.0 && look.z > 0.0);
    }

    #[test]
    fn steep_dive_gains_horizontal_speed_and_shallow_climb_trades_it_for_height() {
        let dive = glide_velocity(Vec3::new(0.0, -0.5, 0.5), glide_input(60.0));
        assert!(dive.z > 0.5 * GLIDE_DRAG[2] - 1.0e-9);
        let climb = glide_velocity(Vec3::new(0.0, 0.0, 1.0), glide_input(-30.0));
        assert!(climb.y > 0.0);
        assert!(climb.z < 1.0);
    }
}
