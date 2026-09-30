//! Non-walking locomotion: ability flight, pose-swimming and elytra gliding.
//!
//! Coefficients follow the public movement-physics notes' BedSim candidates (flight
//! damping, swim steering, glide equations); none is oracle-validated against 1.26.30.

use crate::{
    CollisionWorld, Vec3,
    math::{minecraft_cos, minecraft_sin},
};

use super::{
    AxisCollisions, COLLISION_EPSILON, ControlledTickResult, INPUT_IMPULSE_MULTIPLIER,
    MovementInput, MovementMode, NORMAL_GRAVITY, PlayerState, SimulationError, TickResult,
    apply_relative_movement, collision::resolve_motion, controls, effects,
    environment::SampledEnvironment, scaffolding::ScaffoldingView,
};

const DEFAULT_FLY_SPEED: f64 = 0.05;
const DEFAULT_VERTICAL_FLY_SPEED: f64 = 1.0;
const FLY_SPRINT_MULTIPLIER: f64 = 2.0;
const FLY_ASCEND: f64 = 0.15;
const FLY_DESCEND: f64 = 0.22;
const FLY_FRICTION: f64 = 0.91;
const FLY_HOVER_INPUT_THRESHOLD: f64 = 0.01;
const FLY_HOVER_FRICTION_CREATIVE: f64 = 0.375;
const FLY_HOVER_FRICTION_OTHER: f64 = 0.75;
const FLY_HOVER_VERTICAL_CREATIVE: f64 = 0.375;

const SWIM_HORIZONTAL_DRAG: f64 = 0.9;
const SWIM_VERTICAL_DRAG: f64 = 0.8;
const SWIM_STEER_RATE: f64 = 0.06;
const SWIM_STEER_DIVE_RATE: f64 = 0.085;
const SWIM_DIVE_THRESHOLD: f64 = -0.2;

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
    if input.mode == MovementMode::Riding {
        next.velocity = Vec3::ZERO;
        next.movement = Vec3::ZERO;
        next.jump_delay = 0;
        next.collisions = AxisCollisions::default();
        let result = TickResult {
            tick: next.tick,
            position: next.position,
            velocity: Vec3::ZERO,
            movement: Vec3::ZERO,
            collisions: AxisCollisions::default(),
            on_ground: next.on_ground,
            environment: sampled.movement,
            world_identity: sampled.identity,
        };
        *state = next;
        return Ok(ControlledTickResult {
            tick_result: result,
            controls,
        });
    }
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
            let vertical_speed = input
                .vertical_fly_speed
                .unwrap_or(DEFAULT_VERTICAL_FLY_SPEED);
            if input.jumping && input.sneaking {
                next.velocity.y = 0.0;
            } else {
                let direction = if input.jumping {
                    FLY_ASCEND
                } else if input.sneaking {
                    -FLY_DESCEND
                } else {
                    0.0
                };
                next.velocity.y += vertical_speed * direction;
            }
        }
        MovementMode::Swimming if in_water => {
            apply_relative_movement(
                &mut next.velocity,
                controls.move_vector[0] * INPUT_IMPULSE_MULTIPLIER,
                controls.move_vector[1] * INPUT_IMPULSE_MULTIPLIER,
                input.yaw_degrees,
                super::water_travel_speed(
                    &input,
                    sampled.movement.horizontal_speed_factor,
                    super::depth_strider_blend(input.depth_strider, grounded_at_start),
                ),
            );
            let target = -minecraft_sin(input.pitch_degrees.to_radians());
            let rate = if target < SWIM_DIVE_THRESHOLD {
                SWIM_STEER_DIVE_RATE
            } else {
                SWIM_STEER_RATE
            };
            next.velocity.y += (target - next.velocity.y) * rate;
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
            let horizontal_input = controls.move_vector[0]
                .abs()
                .max(controls.move_vector[1].abs());
            let hovering = horizontal_input < FLY_HOVER_INPUT_THRESHOLD;
            let modifier = match (hovering, input.creative_flight) {
                (false, _) => 1.0,
                (true, true) => FLY_HOVER_FRICTION_CREATIVE,
                (true, false) => FLY_HOVER_FRICTION_OTHER,
            };
            if hovering && input.creative_flight && !input.jumping && !input.sneaking {
                next.velocity.y *= FLY_HOVER_VERTICAL_CREATIVE;
            }
            let retention = FLY_FRICTION * modifier;
            next.velocity.x *= retention;
            next.velocity.z *= retention;
            next.velocity.y *= retention;
        }
        MovementMode::Swimming if in_water => {
            let horizontal = if input.sprinting {
                SWIM_HORIZONTAL_DRAG
            } else {
                super::WATER_DRAG
            };
            next.velocity.x *= horizontal;
            next.velocity.z *= horizontal;
            next.velocity.y *= SWIM_VERTICAL_DRAG;
            effects::apply_vertical(&mut next.velocity.y, input.effects, 0.0, 1.0);
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
