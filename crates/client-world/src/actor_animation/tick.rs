use super::*;

/// Actor state beyond the snapshot that one tick's evaluation reads.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct ActorTickContext {
    pub(crate) is_riding: bool,
    pub(crate) holding_right: bool,
    pub(crate) holding_left: bool,
}

pub(super) fn evaluate_state(
    assets: &RuntimeEntityAssets,
    layout: &VariableLayout,
    state: &ActorRigState,
    actor: &ActorSnapshot,
    context: ActorTickContext,
    tick: u64,
    budget: &mut EvalBudget<'_>,
) -> Result<EvaluatedState, EvalError> {
    let reset = state.reset_pending;
    let anim_tick = if reset {
        0
    } else {
        tick.saturating_sub(state.animation_epoch)
    };
    let life_tick = tick.saturating_sub(state.lifetime_epoch);
    let mut history = if reset {
        VecDeque::with_capacity(MAX_ACTOR_ACTION_HISTORY)
    } else {
        state.history.clone()
    };
    let previous_position = history
        .back()
        .map_or(actor.position, |input| input.position);
    let position_delta = std::array::from_fn(|axis| actor.position[axis] - previous_position[axis]);
    let mut motion = state.motion;
    motion.advance(&MotionInput {
        delta: position_delta,
        riding: context.is_riding,
        player: matches!(actor.kind, ActorKind::Player { .. }),
        yaw: actor.yaw,
        head_yaw: actor.head_yaw,
    });
    let baby_scale = if query::actor_flag(actor, query::FLAG_BABY) {
        1.5
    } else {
        1.0
    };
    if history.len() == MAX_ACTOR_ACTION_HISTORY {
        history.pop_front();
    }
    let input = ActorTickInput {
        position: actor.position,
        position_delta,
        velocity: actor.velocity,
        on_ground: actor.on_ground.unwrap_or(false),
        body_yaw: motion.body_yaw,
        head_yaw: actor.head_yaw,
        pitch: actor.pitch,
        is_riding: context.is_riding,
        distance_moved: motion.distance,
        move_speed: motion.speed.min(1.0) * baby_scale,
    };
    history.push_back(input);
    let evaluator = Evaluator {
        assets,
        layout,
        actor,
        input: &input,
        anim_tick,
        life_tick,
    };
    let rig = assets
        .rig_bindings()
        .get(state.rig_binding)
        .ok_or(EvalError::Invalid)?;
    let mut variables = state.variables.clone();
    if !state.initialized {
        if let Some(script) = rig.initialize {
            evaluator.run(script as usize, &mut variables, 0.0, budget)?;
        }
        // The client seeds the gliding divisor after the pack's initialize script.
        variables.set(layout.engine.gliding_speed_value, 1.0);
    }
    variables.set(layout.engine.is_first_person, 0.0);
    variables.set(layout.engine.attack_time, motion.attack_time());
    variables.set(layout.engine.player_x_rotation, actor.pitch);
    variables.set(
        layout.engine.is_holding_right,
        f32::from(u8::from(context.holding_right)),
    );
    variables.set(
        layout.engine.is_holding_left,
        f32::from(u8::from(context.holding_left)),
    );
    variables.clear_temporaries();
    if let Some(script) = rig.pre_animation {
        evaluator.run(script as usize, &mut variables, 0.0, budget)?;
    }
    let mut controllers = state.controllers.clone();
    if reset {
        for runtime in &mut controllers {
            runtime.state = assets
                .controllers()
                .get(runtime.controller)
                .ok_or(EvalError::Invalid)?
                .initial_state;
        }
    }
    let mut weighted_clips = Vec::new();
    let candidate = assets
        .rig_geometries()
        .get(state.geometry_binding)
        .ok_or(EvalError::Invalid)?;
    let direct_first = candidate.first_animation as usize;
    let direct_end = direct_first
        .checked_add(candidate.animation_count as usize)
        .ok_or(EvalError::Invalid)?;
    for binding in assets
        .rig_animations()
        .get(direct_first..direct_end)
        .ok_or(EvalError::Invalid)?
    {
        budget.charge_work()?;
        let weight = blend_weight(&evaluator, &mut variables, binding.weight, 1.0, budget)?;
        if weight != 0.0 {
            weighted_clips.push((binding.clip as usize, weight));
        }
    }
    let controller_first = candidate.first_controller as usize;
    let controller_end = controller_first
        .checked_add(candidate.controller_count as usize)
        .ok_or(EvalError::Invalid)?;
    for binding in assets
        .rig_controllers()
        .get(controller_first..controller_end)
        .ok_or(EvalError::Invalid)?
    {
        budget.charge_work()?;
        let weight = blend_weight(&evaluator, &mut variables, binding.weight, 1.0, budget)?;
        if weight != 0.0 {
            let mut walk = ControllerWalk {
                evaluator: &evaluator,
                variables: &mut variables,
                controllers: &mut controllers,
                clips: &mut weighted_clips,
                budget,
            };
            walk.evaluate(binding.controller as usize, weight, 0)?;
        }
    }
    let local = sample_clips(
        &evaluator,
        &mut variables,
        state.bones.len(),
        &weighted_clips,
        budget,
    )?;
    compose_pose(&state.bones, &local)
        .map(|pose| EvaluatedState {
            pose,
            controllers,
            history,
            variables,
            motion,
        })
        .ok_or(EvalError::Invalid)
}

fn blend_weight(
    evaluator: &Evaluator<'_>,
    variables: &mut MolangVariables,
    expression: Option<u32>,
    parent: f32,
    budget: &mut EvalBudget<'_>,
) -> Result<f32, EvalError> {
    let weight = match expression {
        Some(expression) => evaluator.run(expression as usize, variables, 0.0, budget)?,
        None => 1.0,
    };
    Ok(if weight.is_finite() {
        parent * weight
    } else {
        0.0
    })
}

/// Advances one controller and collects its active state's clips, descending into nested
/// controllers with the product of the enclosing blend weights.
struct ControllerWalk<'e, 'v, 'b, 'w> {
    evaluator: &'e Evaluator<'e>,
    variables: &'v mut MolangVariables,
    controllers: &'v mut [ControllerState],
    clips: &'v mut Vec<(usize, f32)>,
    budget: &'b mut EvalBudget<'w>,
}

impl ControllerWalk<'_, '_, '_, '_> {
    fn evaluate(&mut self, controller: usize, weight: f32, depth: usize) -> Result<(), EvalError> {
        if depth >= assets::MAX_ENTITY_CONTROLLER_NESTING {
            return Err(EvalError::Invalid);
        }
        let assets = self.evaluator.assets;
        let slot = self
            .controllers
            .iter()
            .position(|runtime| runtime.controller == controller)
            .ok_or(EvalError::Invalid)?;
        self.budget.charge_work()?;
        let state = self.advance(slot)?;
        let controller_state = assets
            .controller_states()
            .get(state)
            .ok_or(EvalError::Invalid)?;
        let first = controller_state.first_animation as usize;
        let end = first
            .checked_add(controller_state.animation_count as usize)
            .ok_or(EvalError::Invalid)?;
        for animation in assets
            .controller_animations()
            .get(first..end)
            .ok_or(EvalError::Invalid)?
        {
            self.budget.charge_work()?;
            let weight = blend_weight(
                self.evaluator,
                self.variables,
                animation.weight,
                weight,
                self.budget,
            )?;
            if weight == 0.0 {
                continue;
            }
            match animation.target {
                EntityControllerAnimationTarget::Clip(clip) => {
                    self.clips.push((clip as usize, weight))
                }
                EntityControllerAnimationTarget::Controller(nested) => {
                    self.evaluate(nested as usize, weight, depth + 1)?;
                }
            }
        }
        Ok(())
    }

    /// Takes at most the bounded number of transitions; returns the absolute state index.
    fn advance(&mut self, slot: usize) -> Result<usize, EvalError> {
        let assets = self.evaluator.assets;
        let runtime = self.controllers[slot];
        let controller = assets
            .controllers()
            .get(runtime.controller)
            .ok_or(EvalError::Invalid)?;
        let mut current = runtime.state;
        loop {
            if current >= controller.state_count {
                return Err(EvalError::Invalid);
            }
            let state_index = controller.first_state as usize + current as usize;
            let state = assets
                .controller_states()
                .get(state_index)
                .ok_or(EvalError::Invalid)?;
            let first = state.first_transition as usize;
            let end = first
                .checked_add(state.transition_count as usize)
                .ok_or(EvalError::Invalid)?;
            let mut target = None;
            for transition in assets
                .controller_transitions()
                .get(first..end)
                .ok_or(EvalError::Invalid)?
            {
                self.budget.charge_work()?;
                let condition = self.evaluator.run(
                    transition.condition as usize,
                    self.variables,
                    0.0,
                    self.budget,
                )?;
                if truthy(condition) {
                    target = Some(transition.target_state);
                    break;
                }
            }
            let Some(target) = target.filter(|_| self.budget.take_transition()) else {
                self.controllers[slot].state = current;
                return Ok(state_index);
            };
            if target >= controller.state_count {
                return Err(EvalError::Invalid);
            }
            if let Some(script) = state.on_exit {
                self.evaluator
                    .run(script as usize, self.variables, 0.0, self.budget)?;
            }
            current = target;
            let entered = assets
                .controller_states()
                .get(controller.first_state as usize + target as usize)
                .ok_or(EvalError::Invalid)?;
            if let Some(script) = entered.on_entry {
                self.evaluator
                    .run(script as usize, self.variables, 0.0, self.budget)?;
            }
        }
    }
}
