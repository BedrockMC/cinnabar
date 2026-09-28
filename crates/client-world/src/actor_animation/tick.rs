use super::{query::FLAG_BABY, *};

/// Actor state beyond the snapshot that one tick's evaluation reads.
#[derive(Clone, Debug, Default)]
pub(crate) struct ActorTickContext {
    pub(crate) is_riding: bool,
    /// Namespaced identifiers of the equipped main-hand and off-hand items.
    pub(crate) main_hand: Option<Arc<str>>,
    pub(crate) off_hand: Option<Arc<str>>,
    /// Namespaced identifier of the actor being ridden.
    pub(crate) ridden: Option<Arc<str>>,
    pub(crate) has_rider: bool,
    pub(crate) has_player_rider: bool,
    /// The local player rendered from its own camera; selects the first-person render controller.
    pub(crate) is_local_first_person: bool,
    /// `[pitch, yaw]` of the view in degrees, for camera-facing billboards.
    pub(crate) camera_rotation: [f32; 2],
}

// Babies' legs cycle faster by this factor; needs independent measurement.
const BABY_MOVE_SPEED_SCALE: f32 = 1.5;

// Gliding divides limb swing by the cubed squared speed over this; needs independent
// measurement.
const GLIDING_SPEED_SQUARED_UNIT: f32 = 0.2;

/// Advances the walk cycle, swing, and body yaw every tick, whether or not the rig's Molang
/// runs, so static and failing rigs still turn and move.
pub(super) fn advance_motion(
    state: &mut ActorRigState,
    actor: &ActorSnapshot,
    context: &ActorTickContext,
) {
    if state.reset_pending {
        state.history.clear();
    }
    let previous_position = state
        .history
        .back()
        .map_or(actor.position, |input| input.position);
    let position_delta = std::array::from_fn(|axis| actor.position[axis] - previous_position[axis]);
    let motion = &mut state.motion;
    motion.advance(&MotionInput {
        delta: position_delta,
        riding: context.is_riding,
        player: matches!(actor.kind, ActorKind::Player { .. }),
        yaw: actor.yaw,
        head_yaw: actor.head_yaw,
    });
    let baby_scale = if query::actor_flag(actor, FLAG_BABY) {
        BABY_MOVE_SPEED_SCALE
    } else {
        1.0
    };
    let item_use_ticks = if query::actor_flag(actor, query::FLAG_USING_ITEM) {
        state
            .history
            .back()
            .map_or(0, |input| input.item_use_ticks)
            .saturating_add(1)
    } else {
        0
    };
    if state.history.len() == MAX_ACTOR_ACTION_HISTORY {
        state.history.pop_front();
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
        walk_distance: motion.walk_distance(),
        item_use_ticks,
    };
    state.history.push_back(input);
}

pub(super) fn evaluate_state(
    assets: &RuntimeEntityAssets,
    layout: &VariableLayout,
    state: &ActorRigState,
    actor: &ActorSnapshot,
    context: &ActorTickContext,
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
    let input = state.history.back().copied().ok_or(EvalError::Invalid)?;
    let motion = state.motion;
    let evaluator = Evaluator {
        assets,
        layout,
        actor,
        input: &input,
        context,
        anim_tick,
        life_tick,
        finished: (false, false),
    };
    let rig = assets
        .rig_bindings()
        .get(state.rig_binding)
        .ok_or(EvalError::Invalid)?;
    let mut variables = state.variables.clone();
    let engine = &layout.engine;
    if !state.initialized {
        for &(slot, value) in &engine.seeded {
            variables.set(Some(slot), value);
        }
        if let Some(script) = rig.initialize {
            evaluator.run(script as usize, &mut variables, 0.0, budget)?;
        }
    }
    apply_engine_variables(engine, &mut variables, actor, context, &input, &motion);
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
            runtime.entered_tick = 0;
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
            weighted_clips.push(WeightedClip {
                clip: binding.clip as usize,
                weight,
                started_tick: 0,
            });
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
            variables,
        })
        .ok_or(EvalError::Invalid)
}

/// Refreshes the variables the client assigns every tick before `pre_animation`.
pub(super) fn apply_engine_variables(
    engine: &EngineSlots,
    variables: &mut MolangVariables,
    actor: &ActorSnapshot,
    context: &ActorTickContext,
    input: &ActorTickInput,
    motion: &MotionState,
) {
    let truth = |value: bool| if value { 1.0 } else { 0.0 };
    let flag = |bit| truth(query::actor_flag(actor, bit));
    let gliding = if query::actor_flag(actor, query::FLAG_GLIDING) {
        let speed_squared = input
            .position_delta
            .iter()
            .map(|axis| axis * axis)
            .sum::<f32>();
        (speed_squared / GLIDING_SPEED_SQUARED_UNIT)
            .powi(3)
            .max(1.0)
    } else {
        1.0
    };
    variables.set(engine.attack_time, motion.attack_time());
    variables.set(engine.gliding_speed_value, gliding);
    variables.set(engine.is_holding_right, truth(context.main_hand.is_some()));
    variables.set(engine.is_holding_left, truth(context.off_hand.is_some()));
    variables.set(engine.is_sneaking, flag(query::FLAG_SNEAKING));
    variables.set(engine.is_blocking, flag(query::FLAG_BLOCKING));
    variables.set(
        engine.damage_nearby_mobs,
        flag(query::FLAG_DAMAGE_NEARBY_MOBS),
    );
    variables.set(engine.is_first_person, truth(context.is_local_first_person));
    variables.set(engine.player_x_rotation, input.pitch);
    // View bobbing is on by default; the first-person walk/breathing bob weigh against this.
    variables.set(engine.bob_animation, 1.0);
}

/// A clip to sample, its blend weight, and the animation tick its controller state began.
pub(super) struct WeightedClip {
    pub(super) clip: usize,
    pub(super) weight: f32,
    pub(super) started_tick: u64,
}

fn blend_weight(
    evaluator: &Evaluator<'_>,
    variables: &mut MolangVariables,
    expression: Option<u32>,
    parent: f32,
    budget: &mut EvalBudget<'_>,
) -> Result<f32, EvalError> {
    let weight = match expression {
        Some(expression) => evaluator.number(expression as usize, variables, 0.0, budget)?,
        None => 1.0,
    };
    // A non-finite weight propagates, as in vanilla, and fails the pose closed.
    Ok(parent * weight)
}

/// Advances one controller and collects its active state's clips, descending into nested
/// controllers with the product of the enclosing blend weights.
struct ControllerWalk<'e, 'v, 'b, 'w> {
    evaluator: &'e Evaluator<'e>,
    variables: &'v mut MolangVariables,
    controllers: &'v mut [ControllerState],
    clips: &'v mut Vec<WeightedClip>,
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
        let started_tick = self.controllers[slot].entered_tick;
        for animation in state_animations(assets, state)? {
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
                EntityControllerAnimationTarget::Clip(clip) => self.clips.push(WeightedClip {
                    clip: clip as usize,
                    weight,
                    started_tick,
                }),
                EntityControllerAnimationTarget::Controller(nested) => {
                    self.evaluate(nested as usize, weight, depth + 1)?;
                }
            }
        }
        Ok(())
    }
}

fn state_animations(
    assets: &RuntimeEntityAssets,
    state: usize,
) -> Result<&[assets::EntityControllerAnimation], EvalError> {
    {
        let state = assets
            .controller_states()
            .get(state)
            .ok_or(EvalError::Invalid)?;
        let first = state.first_animation as usize;
        let end = first
            .checked_add(state.animation_count as usize)
            .ok_or(EvalError::Invalid)?;
        assets
            .controller_animations()
            .get(first..end)
            .ok_or(EvalError::Invalid)
    }
}

impl ControllerWalk<'_, '_, '_, '_> {
    /// Whether every and any clip of a state has played through once since it was entered.
    fn finished(&self, state: usize, entered_tick: u64) -> Result<(bool, bool), EvalError> {
        let elapsed = self.evaluator.anim_tick.saturating_sub(entered_tick) as f32 * 0.05;
        let mut all = true;
        let mut any = false;
        for animation in state_animations(self.evaluator.assets, state)? {
            let EntityControllerAnimationTarget::Clip(clip) = animation.target else {
                continue;
            };
            let clip = self
                .evaluator
                .assets
                .animation_clips()
                .get(clip as usize)
                .ok_or(EvalError::Invalid)?;
            let done = elapsed >= clip.length_seconds.get();
            all &= done;
            any |= done;
        }
        Ok((all, any))
    }

    /// Takes at most the bounded number of transitions; returns the absolute state index.
    fn advance(&mut self, slot: usize) -> Result<usize, EvalError> {
        let assets = self.evaluator.assets;
        let runtime = self.controllers[slot];
        let controller = assets
            .controllers()
            .get(runtime.controller)
            .ok_or(EvalError::Invalid)?;
        let (mut current, mut entered_tick) = (runtime.state, runtime.entered_tick);
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
            let transitions = assets
                .controller_transitions()
                .get(first..end)
                .ok_or(EvalError::Invalid)?;
            let evaluator = Evaluator {
                finished: if transitions.is_empty() {
                    (false, false)
                } else {
                    self.finished(state_index, entered_tick)?
                },
                ..*self.evaluator
            };
            let mut target = None;
            for transition in transitions {
                self.budget.charge_work()?;
                let condition = evaluator.run(
                    transition.condition as usize,
                    self.variables,
                    0.0,
                    self.budget,
                )?;
                if condition.truthy() {
                    target = Some(transition.target_state);
                    break;
                }
            }
            let Some(target) = target.filter(|_| self.budget.take_transition()) else {
                self.controllers[slot].state = current;
                self.controllers[slot].entered_tick = entered_tick;
                return Ok(state_index);
            };
            if target >= controller.state_count {
                return Err(EvalError::Invalid);
            }
            if let Some(script) = state.on_exit {
                evaluator.run(script as usize, self.variables, 0.0, self.budget)?;
            }
            current = target;
            entered_tick = self.evaluator.anim_tick;
            let entered = assets
                .controller_states()
                .get(controller.first_state as usize + target as usize)
                .ok_or(EvalError::Invalid)?;
            if let Some(script) = entered.on_entry {
                evaluator.run(script as usize, self.variables, 0.0, self.budget)?;
            }
        }
    }
}
