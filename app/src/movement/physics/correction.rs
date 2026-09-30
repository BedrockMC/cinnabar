//! Correction rewind and replay for the local physics controller.

use super::*;

impl LocalPhysicsController {
    pub(in crate::movement) fn apply_correction(
        &mut self,
        network_position: [f32; 3],
        tick: u64,
        on_ground: bool,
        velocity: Option<[f32; 3]>,
        mode: PhysicsCorrectionMode,
        confirmation: Option<&PhysicsCorrectionConfirmation>,
        world: &impl CollisionWorld,
    ) -> Result<PhysicsCorrectionPlan, PhysicsCorrectionError> {
        if !network_position.into_iter().all(f32::is_finite) {
            return Err(PhysicsCorrectionError::InvalidAnchor);
        }
        let velocity = velocity
            .filter(|velocity| super::timeline::motion_is_simulable(*velocity))
            .map(|velocity| {
                Vec3::new(
                    f64::from(velocity[0]),
                    f64::from(velocity[1]),
                    f64::from(velocity[2]),
                )
            });
        self.corrections_applied = self.corrections_applied.saturating_add(1);
        if matches!(mode, PhysicsCorrectionMode::Snap) {
            self.reanchor_network_position_before_advance(network_position, tick, on_ground);
            if let (Some(velocity), Some(state)) = (velocity, self.state.as_mut()) {
                state.velocity = velocity;
            }
            return Ok(PhysicsCorrectionPlan {
                outcome: PhysicsCorrectionOutcome::Snapped { tick },
                corrected_tick: tick,
                final_tick: tick,
                final_position: network_position,
                replayed_samples: Vec::new(),
            });
        }

        if self.state.is_none() {
            return Err(PhysicsCorrectionError::NotRetained { tick });
        }

        let current_tick = self
            .state
            .as_ref()
            .expect("active correction checked for local state")
            .tick;
        if tick > current_tick {
            return Err(PhysicsCorrectionError::NotRetained { tick });
        }
        let Some(mut corrected) = self.history.state_at(tick).cloned() else {
            return Err(PhysicsCorrectionError::NotRetained { tick });
        };
        if !self.sample_history.iter().any(|sample| sample.tick == tick) {
            return Err(PhysicsCorrectionError::NotRetained { tick });
        }

        let feet = Vec3::new(
            f64::from(network_position[0]),
            f64::from(network_position[1] - PLAYER_NETWORK_OFFSET),
            f64::from(network_position[2]),
        );
        // Vanilla's correction input writes both position and StateVector
        // motion into the corrected frame before replaying later inputs.
        corrected.position = feet;
        corrected.on_ground = on_ground;
        // Axis collisions describe the motion that produced a position, so they
        // cannot be recomputed from a corrected anchor. They are retained only
        // when a bounded transport-success record shows that the correction
        // exactly matches the network position this client sent for that tick,
        // the retained sample used that same immutable collision identity, and
        // every chunk in that identity is still loaded at the same revision.
        // Cinnabar provisionally interprets that combination as confirmation of
        // the motion behind the position; this is a client replay policy, not
        // an established vanilla or protocol guarantee. Retaining the flags
        // avoids stuttering a legitimate wall climb on matching corrections.
        // Any missing proof or mismatch clears the flags and keeps the discrete
        // climb branch closed. An upward velocity produced while a stale
        // horizontal collision was retained is the same unconfirmed ladder
        // response, so it is cleared with those flags. Identity query failure
        // is semantic unavailability and does not disconnect. The position
        // comparison is exact in the sent `f32` network space because that is
        // the serialized position available to compare. The loss is bounded
        // to the corrected tick: `Simulator::tick` re-derives collisions.
        let retained_sample = self
            .sample_history
            .iter()
            .find(|sample| sample.tick == tick)
            .expect("retained correction sample was checked");
        let server_confirmed_prediction = confirmation.is_some_and(|confirmation| {
            confirmation.position == network_position
                && retained_sample.position == network_position
                && confirmation.world_identity == retained_sample.world_identity
                && collision_identity_is_current(world, feet, &confirmation.world_identity)
                    .unwrap_or(false)
        });
        if !server_confirmed_prediction {
            if (corrected.collisions.x || corrected.collisions.z) && corrected.velocity.y > 0.0 {
                corrected.velocity.y = 0.0;
            }
            corrected.collisions = sim::AxisCollisions::default();
        }
        if let Some(velocity) = velocity {
            corrected.velocity = velocity;
        }
        self.replay_from_corrected(tick, corrected, Some(network_position), world)
    }

    /// Re-simulates every retained tick after `tick` from its unchanged state so
    /// timeline edits recorded after it (motion, attributes, flags) take effect.
    pub(in crate::movement) fn replay_retained_from(
        &mut self,
        tick: u64,
        world: &impl CollisionWorld,
    ) -> Result<PhysicsCorrectionPlan, PhysicsCorrectionError> {
        let current_tick = self
            .state
            .as_ref()
            .ok_or(PhysicsCorrectionError::NotRetained { tick })?
            .tick;
        if tick > current_tick || !self.sample_history.iter().any(|sample| sample.tick == tick) {
            return Err(PhysicsCorrectionError::NotRetained { tick });
        }
        let Some(corrected) = self.history.state_at(tick).cloned() else {
            return Err(PhysicsCorrectionError::NotRetained { tick });
        };
        self.replay_from_corrected(tick, corrected, None, world)
    }

    fn replay_from_corrected(
        &mut self,
        tick: u64,
        corrected: PlayerState,
        corrected_network_position: Option<[f32; 3]>,
        world: &impl CollisionWorld,
    ) -> Result<PhysicsCorrectionPlan, PhysicsCorrectionError> {
        let on_ground = corrected.on_ground;
        let feet = corrected.position;
        let motion_overlays: Vec<sim::MotionOverlay> =
            self.server_motions.iter().copied().collect();
        // The replay starts from this exact anchor state; capture its cooldown
        // before consumption so the initiation fold seeds identically.
        let anchor_jump_delay = corrected.jump_delay;
        let (replay, replayed_ticks) = self
            .history
            .rewind_and_replay_with_controls(
                self.state
                    .as_mut()
                    .expect("active correction checked for local state"),
                corrected,
                &self.simulator,
                world,
                &motion_overlays,
            )
            .map_err(|_| PhysicsCorrectionError::ReplayFailed)?;

        if replayed_ticks.len() != replay.replayed_ticks {
            return Err(PhysicsCorrectionError::ReplayFailed);
        }
        // Rebuilds the processed jump state across the replayed range exactly
        // like velocity is rebuilt: initiations are facts of the replayed
        // timeline (the same retained request edges re-fed from the corrected
        // anchor), so [`ReplayJumpArcFold`] recomputes them with the
        // simulator's own consumption rule instead of trusting records a
        // contradicted prediction may have left stale in either direction.
        // The entering window follows that anchor: a server-reported ground
        // contact outranks a retained initiation (the correction just
        // contradicted this client's takeoff), while an airborne anchor keeps
        // the recorded arc as the un-replayed continuation of earlier ticks.
        let mut jump_fold = {
            let corrected_sample = self
                .sample_history
                .iter()
                .find(|sample| sample.tick == tick)
                .expect("retained correction sample was checked");
            ReplayJumpArcFold::seed(
                on_ground,
                anchor_jump_delay,
                corrected_sample.processed.jump_initiated,
                corrected_sample.processed.jump_arc_active,
            )
        };
        let mut replayed_samples = Vec::with_capacity(replayed_ticks.len());
        for output in replayed_ticks {
            let result = output.tick_result;
            let Some(retained) = self
                .sample_history
                .iter_mut()
                .find(|sample| sample.tick == result.tick)
            else {
                return Err(PhysicsCorrectionError::NotRetained { tick: result.tick });
            };
            if retained.world_identity != result.world_identity {
                return Err(PhysicsCorrectionError::WorldIdentityMismatch { tick: result.tick });
            }
            retained.position = [
                result.position.x as f32,
                result.position.y as f32 + PLAYER_NETWORK_OFFSET,
                result.position.z as f32,
            ];
            retained.movement = [
                result.movement.x as f32,
                result.movement.y as f32,
                result.movement.z as f32,
            ];
            retained.velocity = [
                result.velocity.x as f32,
                result.velocity.y as f32,
                result.velocity.z as f32,
            ];
            retained.move_vector = [
                -output.controls.move_vector[0] as f32,
                output.controls.move_vector[1] as f32,
            ];
            retained.horizontal_collision = result.collisions.x || result.collisions.z;
            retained.vertical_collision = result.collisions.y;
            retained.grounded_after_tick = result.on_ground;
            // The replay fed this input verbatim, mirroring the simulator's
            // own per-tick consumption.
            let Some(frame_input) = self.history.input_at(result.tick) else {
                return Err(PhysicsCorrectionError::NotRetained { tick: result.tick });
            };
            let (initiated, arc_active) = jump_fold.step(frame_input, result.on_ground);
            retained.sneaking = frame_input.sneaking;
            retained.sprinting = frame_input.sprinting;
            retained.processed.sneaking = frame_input.sneaking;
            retained.processed.sprinting = frame_input.sprinting;
            retained.processed.mode = frame_input.mode;
            retained.processed.direction_flags = Some(super::super::encoding::direction_flags([
                -frame_input.strafe as f32,
                frame_input.forward as f32,
            ]));
            retained.processed.jump_initiated = initiated;
            retained.processed.jump_arc_active = arc_active;
            replayed_samples.push(retained.clone());
        }
        self.processed_jump_arc_active = jump_fold.arc_active();
        let corrected_world_identity = {
            let corrected_sample = self
                .sample_history
                .iter_mut()
                .find(|sample| sample.tick == tick)
                .expect("retained correction sample was checked");
            if let Some(position) = corrected_network_position {
                corrected_sample.position = position;
            }
            corrected_sample.world_identity.clone()
        };

        let state = self
            .state
            .as_ref()
            .expect("successful replay retains local state");
        let final_tick = state.tick;
        let final_position = [
            state.position.x as f32,
            state.position.y as f32 + PLAYER_NETWORK_OFFSET,
            state.position.z as f32,
        ];
        self.previous_position = if final_tick == tick {
            feet
        } else {
            self.history
                .state_at(final_tick.saturating_sub(1))
                .map_or(feet, |previous| previous.position)
        };
        self.accumulated_seconds = 0.0;
        self.last_world_identity = replayed_samples
            .last()
            .map(|sample| sample.world_identity.clone())
            .or(Some(corrected_world_identity));
        // A replay re-anchors the corrected tick and rebuilds later ticks from
        // it; that landing can sit inside solids just like a hard anchor. Re-arm
        // the depenetration probe so the next tick pushes the anchor out
        // positionally instead of streaming an embedded pose indefinitely.
        self.anchor_state.rearm();

        Ok(PhysicsCorrectionPlan {
            outcome: PhysicsCorrectionOutcome::Replayed {
                corrected_tick: replay.corrected_tick,
                replayed_ticks: replay.replayed_ticks,
            },
            corrected_tick: tick,
            final_tick,
            final_position,
            replayed_samples,
        })
    }
}

fn collision_identity_is_current(
    world: &impl CollisionWorld,
    corrected_feet: Vec3,
    expected: &WorldCollisionIdentity,
) -> Result<bool, sim::WorldQueryError> {
    let y = checked_block_coordinate(corrected_feet.y)?;
    let mut current: Option<WorldCollisionIdentity> = None;
    if expected.chunks.is_empty() {
        let block = [
            checked_block_coordinate(corrected_feet.x)?,
            y,
            checked_block_coordinate(corrected_feet.z)?,
        ];
        current = Some(world.block_physics(block)?.identity);
    } else {
        for revision in &expected.chunks {
            let Some(x) = revision.chunk.x.checked_mul(16) else {
                return Err(sim::WorldQueryError::CoordinateOutOfRange);
            };
            let Some(z) = revision.chunk.z.checked_mul(16) else {
                return Err(sim::WorldQueryError::CoordinateOutOfRange);
            };
            let identity = world.block_physics([x, y, z])?.identity;
            current = Some(match current {
                None => identity,
                Some(previous) => previous.merge(&identity)?,
            });
        }
    }
    Ok(current.as_ref() == Some(expected))
}

fn checked_block_coordinate(value: f64) -> Result<i32, sim::WorldQueryError> {
    let value = value.floor();
    if value < f64::from(i32::MIN) || value > f64::from(i32::MAX) {
        return Err(sim::WorldQueryError::CoordinateOutOfRange);
    }
    Ok(value as i32)
}
