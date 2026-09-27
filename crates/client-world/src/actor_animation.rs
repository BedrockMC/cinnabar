use std::{
    collections::{BTreeMap, HashMap, VecDeque},
    sync::Arc,
};

use assets::{
    EntityAnimationInterpolation, EntityAnimationLoop, EntityAssetKind,
    EntityControllerAnimationTarget, EntityGeometryBone, EntityRigFallback, MolangOp,
    RuntimeEntityAssets, validate_entity_geometry_inheritance,
};
use protocol::{ActorKind, ActorMetadataValue};

use crate::actor_store::ActorSnapshot;

pub const MAX_RUNTIME_BONES_PER_RIG: usize = 96;
pub const MAX_CONTROLLER_TRANSITIONS_PER_TICK: usize = 8;
pub const MAX_MOLANG_OPS_PER_ACTOR_TICK: usize = 4_096;
pub const MAX_MOLANG_OPS_PER_WORLD_TICK: usize = 262_144;
pub const MAX_MOLANG_OPS_PER_RENDER_FRAME: usize = 0;
pub const MAX_ACTOR_ACTION_HISTORY: usize = 32;
const MAX_RUNTIME_POSE_WORK_PER_ACTOR_TICK: usize = 4_096;
const MAX_RUNTIME_BINDINGS_PER_RIG: usize = 4_096;

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct ActorLifetimeId {
    pub session_id: u64,
    pub dimension: i32,
    pub runtime_id: u64,
    pub spawn_revision: u64,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct EntityRigId(pub u32);

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct BoneTransform {
    pub rotation: [f32; 4],
    pub translation_scale: [f32; 4],
    /// Non-uniform scale in the bone's own frame; `[1; 3]` when the scale is uniform.
    pub axis_scale: [f32; 3],
}

#[derive(Clone, Copy, Debug)]
pub struct ActorRigSnapshot<'a> {
    pub actor: ActorLifetimeId,
    pub rig: EntityRigId,
    pub previous: &'a [BoneTransform],
    pub current: &'a [BoneTransform],
    /// Immutable authored rest transforms from the exact resolved geometry.
    pub rest: &'a [BoneTransform],
    /// Actual fixed-tick observation of this lifetime, independent of pose evaluation.
    pub rest_completed_tick: u64,
    pub rest_reset_generation: u64,
    pub completed_tick: u64,
    pub reset_generation: u64,
    pub fallback: EntityRigFallback,
    /// Authored uniform model scale about the feet origin.
    pub scale: f32,
    /// Body yaw in degrees at the previous and current completed tick.
    pub previous_body_yaw: f32,
    pub body_yaw: f32,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ActorAnimationStats {
    pub evaluated_molang_ops: u64,
    pub actor_budget_exhaustions: u64,
    pub world_budget_exhaustions: u64,
    pub frozen_actors: u64,
    pub unrigged_spawns: u64, // spawns with entity assets loaded but no compiled rig
}

#[derive(Debug)]
pub(crate) struct ActorAnimationStore {
    assets: Option<Arc<RuntimeEntityAssets>>,
    layout: Arc<VariableLayout>,
    rigs: BTreeMap<ActorLifetimeId, ActorRigState>,
    runtime_to_lifetime: HashMap<u64, ActorLifetimeId>,
    /// First actor the world budget skipped last tick, where the next tick starts.
    first_starved: Option<ActorLifetimeId>,
    completed_tick: u64,
    next_reset_generation: u64,
    next_rest_reset_generation: u64,
    stats: ActorAnimationStats,
}

#[derive(Debug)]
struct ActorRigState {
    rig: EntityRigId,
    rig_binding: usize,
    geometry_binding: usize,
    bones: Vec<RuntimeBone>,
    controllers: Vec<ControllerState>,
    previous: Vec<BoneTransform>,
    current: Vec<BoneTransform>,
    rest: Vec<BoneTransform>,
    rest_completed_tick: u64,
    rest_reset_generation: u64,
    rest_reset_pending: bool,
    reset_generation: u64,
    reset_pending: bool,
    lifetime_epoch: u64,
    animation_epoch: u64,
    completed_tick: u64,
    fallback: EntityRigFallback,
    history: VecDeque<ActorTickInput>,
    variables: MolangVariables,
    initialized: bool,
    motion: MotionState,
}

#[derive(Clone, Debug)]
struct RuntimeBone {
    parent: Option<usize>,
    pivot: [f32; 3],
    rotation: [f32; 3],
}

#[derive(Clone, Copy, Debug)]
struct ControllerState {
    controller: usize,
    state: u16,
    /// Animation tick the current state was entered, where its clips start.
    entered_tick: u64,
}

#[derive(Clone, Copy, Debug, Default)]
struct ActorTickInput {
    position: [f32; 3],
    position_delta: [f32; 3],
    velocity: [f32; 3],
    on_ground: bool,
    body_yaw: f32,
    head_yaw: f32,
    pitch: f32,
    is_riding: bool,
    distance_moved: f32,
    move_speed: f32,
    walk_distance: f32,
}

struct EvaluatedState {
    pose: Vec<BoneTransform>,
    controllers: Vec<ControllerState>,
    variables: MolangVariables,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum EvalError {
    ActorBudget,
    WorldBudget,
    Invalid,
}

struct EvalBudget<'a> {
    actor_left: usize,
    world_left: &'a mut usize,
    work_left: usize,
    transitions_left: usize,
    used: usize,
}

impl EvalBudget<'_> {
    fn charge(&mut self) -> Result<(), EvalError> {
        if self.actor_left == 0 {
            return Err(EvalError::ActorBudget);
        }
        if *self.world_left == 0 {
            return Err(EvalError::WorldBudget);
        }
        self.actor_left -= 1;
        *self.world_left -= 1;
        self.used += 1;
        Ok(())
    }

    fn charge_work(&mut self) -> Result<(), EvalError> {
        if self.work_left == 0 {
            return Err(EvalError::ActorBudget);
        }
        self.work_left -= 1;
        Ok(())
    }

    fn take_transition(&mut self) -> bool {
        if self.transitions_left == 0 {
            return false;
        }
        self.transitions_left -= 1;
        true
    }
}

impl ActorAnimationStore {
    pub(crate) fn diagnostic() -> Self {
        Self::new(None)
    }

    pub(crate) fn with_assets(assets: Arc<RuntimeEntityAssets>) -> Self {
        Self::new(Some(assets))
    }

    fn new(assets: Option<Arc<RuntimeEntityAssets>>) -> Self {
        Self {
            layout: Arc::new(
                assets
                    .as_deref()
                    .map(VariableLayout::new)
                    .unwrap_or_default(),
            ),
            assets,
            rigs: BTreeMap::new(),
            runtime_to_lifetime: HashMap::new(),
            first_starved: None,
            completed_tick: 0,
            next_reset_generation: 1,
            next_rest_reset_generation: 1,
            stats: ActorAnimationStats::default(),
        }
    }

    pub(crate) fn clear(&mut self) {
        self.rigs.clear();
        self.runtime_to_lifetime.clear();
        self.completed_tick = 0;
        self.bump_generation();
    }

    pub(crate) fn remove_runtime(&mut self, runtime_id: u64) {
        if let Some(lifetime) = self.runtime_to_lifetime.remove(&runtime_id) {
            self.rigs.remove(&lifetime);
        }
    }

    pub(crate) fn insert(&mut self, session_id: u64, dimension: i32, actor: &ActorSnapshot) {
        self.remove_runtime(actor.runtime_id);
        let Some(assets) = self.assets.clone() else {
            return;
        };
        let lifetime = ActorLifetimeId {
            session_id,
            dimension,
            runtime_id: actor.runtime_id,
            spawn_revision: actor.spawn_revision,
        };
        let Some(mut state) = resolve_rig(&assets, &self.layout, actor, self.completed_tick) else {
            self.stats.unrigged_spawns = self.stats.unrigged_spawns.saturating_add(1);
            return;
        };
        state.reset_generation = self.next_reset_generation;
        state.rest_reset_generation = self.take_rest_generation().unwrap_or(0);
        self.bump_generation();
        self.runtime_to_lifetime.insert(actor.runtime_id, lifetime);
        self.rigs.insert(lifetime, state);
    }

    pub(crate) fn mark_reset(&mut self, runtime_id: u64) {
        let Some(lifetime) = self.runtime_to_lifetime.get(&runtime_id) else {
            return;
        };
        if let Some(state) = self.rigs.get_mut(lifetime) {
            state.reset_pending = true;
            state.rest_reset_pending = true;
        }
    }

    /// Restarts the arm swing whose progress feeds `variable.attack_time`.
    pub(crate) fn start_swing(&mut self, runtime_id: u64) {
        let Some(lifetime) = self.runtime_to_lifetime.get(&runtime_id) else {
            return;
        };
        if let Some(state) = self.rigs.get_mut(lifetime) {
            state.motion.start_swing();
        }
    }

    pub(crate) fn advance_tick(
        &mut self,
        actors: &HashMap<u64, ActorSnapshot>,
        context: impl Fn(&ActorSnapshot) -> ActorTickContext,
    ) {
        self.completed_tick = self.completed_tick.saturating_add(1);
        let Some(assets) = self.assets.clone() else {
            return;
        };
        let mut world_left = MAX_MOLANG_OPS_PER_WORLD_TICK;
        // Start where the world budget ran out last tick so no actor starves every tick.
        let lifetimes = match self.first_starved.take() {
            Some(start) => self
                .rigs
                .range(start..)
                .chain(self.rigs.range(..start))
                .map(|(lifetime, _)| *lifetime)
                .collect::<Vec<_>>(),
            None => self.rigs.keys().copied().collect(),
        };
        let mut starved = None;
        for lifetime in lifetimes {
            let Some(actor) = actors.get(&lifetime.runtime_id) else {
                continue;
            };
            let Some(state) = self.rigs.get_mut(&lifetime) else {
                continue;
            };
            // Observe ownership before any evaluation budget branch. A failed
            // animation cannot starve static publication for this or later actors.
            if actor.runtime_id == lifetime.runtime_id
                && actor.spawn_revision == lifetime.spawn_revision
                && self.runtime_to_lifetime.get(&lifetime.runtime_id) == Some(&lifetime)
            {
                if state.rest_reset_pending {
                    if let Some(next) = self.next_rest_reset_generation.checked_add(1) {
                        state.rest_reset_generation = self.next_rest_reset_generation;
                        self.next_rest_reset_generation = next;
                        state.rest_reset_pending = false;
                    } else {
                        state.rest_reset_generation = 0;
                    }
                }
                state.rest_completed_tick =
                    if state.rest_reset_generation != 0 && !state.rest_reset_pending {
                        self.completed_tick
                    } else {
                        0
                    };
            } else {
                state.rest_completed_tick = 0;
            }
            let context = context(actor);
            advance_motion(state, actor, &context);
            if state.fallback == EntityRigFallback::GeometryOnly {
                state.previous.clone_from(&state.current);
                if state.reset_pending {
                    state.reset_pending = false;
                    state.reset_generation = self.next_reset_generation;
                    self.next_reset_generation = self.next_reset_generation.saturating_add(1);
                    state.animation_epoch = self.completed_tick;
                }
                state.completed_tick = self.completed_tick;
                continue;
            }
            if world_left == 0 {
                self.stats.world_budget_exhaustions =
                    self.stats.world_budget_exhaustions.saturating_add(1);
                self.stats.frozen_actors = self.stats.frozen_actors.saturating_add(1);
                starved.get_or_insert(lifetime);
                continue;
            }
            let mut budget = EvalBudget {
                actor_left: MAX_MOLANG_OPS_PER_ACTOR_TICK,
                world_left: &mut world_left,
                work_left: MAX_RUNTIME_POSE_WORK_PER_ACTOR_TICK,
                transitions_left: MAX_CONTROLLER_TRANSITIONS_PER_TICK,
                used: 0,
            };
            let result = evaluate_state(
                &assets,
                &self.layout,
                state,
                actor,
                &context,
                self.completed_tick,
                &mut budget,
            );
            self.stats.evaluated_molang_ops = self
                .stats
                .evaluated_molang_ops
                .saturating_add(budget.used as u64);
            match result {
                Ok(evaluated) => {
                    state.controllers = evaluated.controllers;
                    state.variables = evaluated.variables;
                    state.initialized = true;
                    if state.reset_pending {
                        state.previous.clone_from(&evaluated.pose);
                        state.current = evaluated.pose;
                        state.reset_pending = false;
                        state.reset_generation = self.next_reset_generation;
                        self.next_reset_generation = self.next_reset_generation.saturating_add(1);
                        state.animation_epoch = self.completed_tick;
                    } else {
                        state.previous = std::mem::replace(&mut state.current, evaluated.pose);
                    }
                    state.completed_tick = self.completed_tick;
                }
                Err(EvalError::ActorBudget) => {
                    self.stats.actor_budget_exhaustions =
                        self.stats.actor_budget_exhaustions.saturating_add(1);
                    self.stats.frozen_actors = self.stats.frozen_actors.saturating_add(1);
                }
                Err(EvalError::WorldBudget) => {
                    self.stats.world_budget_exhaustions =
                        self.stats.world_budget_exhaustions.saturating_add(1);
                    self.stats.frozen_actors = self.stats.frozen_actors.saturating_add(1);
                    starved.get_or_insert(lifetime);
                }
                Err(EvalError::Invalid) => {
                    self.stats.frozen_actors = self.stats.frozen_actors.saturating_add(1);
                }
            }
        }
        self.first_starved = starved;
    }

    pub(crate) fn get(&self, runtime_id: u64) -> Option<ActorRigSnapshot<'_>> {
        let lifetime = *self.runtime_to_lifetime.get(&runtime_id)?;
        self.snapshot(lifetime, self.rigs.get(&lifetime)?)
    }

    pub(crate) fn snapshots(&self) -> Vec<ActorRigSnapshot<'_>> {
        self.rigs
            .iter()
            .filter_map(|(&lifetime, state)| self.snapshot(lifetime, state))
            .collect()
    }

    pub(crate) const fn stats(&self) -> ActorAnimationStats {
        self.stats
    }

    fn snapshot<'a>(
        &'a self,
        actor: ActorLifetimeId,
        state: &'a ActorRigState,
    ) -> Option<ActorRigSnapshot<'a>> {
        if state.previous.len() != state.current.len() {
            return None;
        }
        Some(ActorRigSnapshot {
            actor,
            rig: state.rig,
            previous: &state.previous,
            current: &state.current,
            rest: &state.rest,
            rest_completed_tick: if state.rest_reset_pending {
                0
            } else {
                state.rest_completed_tick
            },
            rest_reset_generation: state.rest_reset_generation,
            completed_tick: state.completed_tick,
            reset_generation: state.reset_generation,
            fallback: state.fallback,
            scale: self
                .assets
                .as_ref()
                .and_then(|assets| assets.rig_bindings().get(state.rig_binding))
                .map_or(1.0, |rig| rig.scale.get()),
            previous_body_yaw: state.motion.previous_body_yaw,
            body_yaw: state.motion.body_yaw,
        })
    }

    fn bump_generation(&mut self) {
        self.next_reset_generation = self.next_reset_generation.saturating_add(1);
    }

    fn take_rest_generation(&mut self) -> Option<u64> {
        let next = self.next_rest_reset_generation.checked_add(1)?;
        let generation = self.next_rest_reset_generation;
        self.next_rest_reset_generation = next;
        Some(generation)
    }
}

fn resolve_rig(
    assets: &RuntimeEntityAssets,
    layout: &VariableLayout,
    actor: &ActorSnapshot,
    completed_tick: u64,
) -> Option<ActorRigState> {
    let identifier = match &actor.kind {
        ActorKind::Player { .. } => "minecraft:player",
        ActorKind::Entity { identifier } => identifier,
    };
    let entity_symbol = assets
        .symbol_candidates(EntityAssetKind::Entity, identifier)
        .first()?;
    let entity_symbol_index = assets
        .symbols()
        .iter()
        .position(|symbol| std::ptr::eq(symbol, entity_symbol))?;
    let rig_binding = assets
        .rig_bindings()
        .iter()
        .position(|rig| rig.entity_symbol as usize == entity_symbol_index)?;
    let rig = &assets.rig_bindings()[rig_binding];
    let first = rig.first_geometry as usize;
    let end = first.checked_add(rig.geometry_count as usize)?;
    let candidates = assets.rig_geometries().get(first..end)?;
    let mut world_left = MAX_MOLANG_OPS_PER_ACTOR_TICK;
    let mut budget = EvalBudget {
        actor_left: MAX_MOLANG_OPS_PER_ACTOR_TICK,
        world_left: &mut world_left,
        work_left: MAX_RUNTIME_POSE_WORK_PER_ACTOR_TICK,
        transitions_left: MAX_CONTROLLER_TRANSITIONS_PER_TICK,
        used: 0,
    };
    let mut candidate_offset = 0;
    let input = ActorTickInput {
        position: actor.position,
        velocity: actor.velocity,
        on_ground: actor.on_ground.unwrap_or(false),
        body_yaw: actor.body_yaw,
        head_yaw: actor.head_yaw,
        pitch: actor.pitch,
        ..ActorTickInput::default()
    };
    let context = ActorTickContext::default();
    let evaluator = Evaluator {
        assets,
        layout,
        actor,
        input: &input,
        context: &context,
        anim_tick: 0,
        life_tick: 0,
        finished: (false, false),
    };
    let mut variables = layout.fresh(actor.runtime_id ^ actor.spawn_revision.rotate_left(32));
    for (offset, candidate) in candidates.iter().enumerate().skip(1) {
        let selected = evaluator
            .run(
                candidate.condition? as usize,
                &mut variables,
                0.0,
                &mut budget,
            )
            .ok()?;
        if selected.truthy() {
            candidate_offset = offset;
            break;
        }
    }
    let geometry_binding = first + candidate_offset;
    let candidate = &assets.rig_geometries()[geometry_binding];
    if candidate.animation_count as usize + candidate.controller_count as usize
        > MAX_RUNTIME_BINDINGS_PER_RIG
    {
        return None;
    }
    let bones = resolve_bones(assets, candidate.geometry as usize)?;
    let current = compose_pose(&bones, &[])?;
    let controller_first = candidate.first_controller as usize;
    let controller_end = controller_first.checked_add(candidate.controller_count as usize)?;
    let mut controllers = Vec::new();
    for binding in assets
        .rig_controllers()
        .get(controller_first..controller_end)?
    {
        collect_controllers(assets, binding.controller as usize, 0, &mut controllers)?;
    }
    Some(ActorRigState {
        // The renderer needs the resolved geometry candidate, not only the
        // entity-level binding that may contain several candidates.
        rig: EntityRigId(geometry_binding as u32),
        rig_binding,
        geometry_binding,
        bones,
        controllers,
        previous: current.clone(),
        rest: current.clone(),
        rest_completed_tick: 0,
        rest_reset_generation: 0,
        rest_reset_pending: false,
        current,
        reset_generation: 0,
        reset_pending: false,
        lifetime_epoch: completed_tick,
        animation_epoch: completed_tick,
        completed_tick,
        fallback: rig.fallback,
        history: VecDeque::with_capacity(MAX_ACTOR_ACTION_HISTORY),
        variables,
        initialized: false,
        motion: MotionState::spawn(actor.body_yaw, actor.head_yaw),
    })
}

/// Adds one runtime state per controller reachable from a rig root, each once.
fn collect_controllers(
    assets: &RuntimeEntityAssets,
    controller: usize,
    depth: usize,
    output: &mut Vec<ControllerState>,
) -> Option<()> {
    if depth >= assets::MAX_ENTITY_CONTROLLER_NESTING {
        return None;
    }
    if output
        .iter()
        .any(|runtime| runtime.controller == controller)
    {
        return Some(());
    }
    let compiled = assets.controllers().get(controller)?;
    output.push(ControllerState {
        controller,
        state: compiled.initial_state,
        entered_tick: 0,
    });
    let states = assets.controller_states().get(
        compiled.first_state as usize
            ..compiled.first_state as usize + compiled.state_count as usize,
    )?;
    for state in states {
        let animations = assets.controller_animations().get(
            state.first_animation as usize
                ..state.first_animation as usize + state.animation_count as usize,
        )?;
        for animation in animations {
            if let EntityControllerAnimationTarget::Controller(nested) = animation.target {
                collect_controllers(assets, nested as usize, depth + 1, output)?;
            }
        }
    }
    Some(())
}

fn resolve_bones(assets: &RuntimeEntityAssets, geometry_index: usize) -> Option<Vec<RuntimeBone>> {
    let parents = validate_entity_geometry_inheritance(assets.geometries()).ok()?;
    let mut chain = Vec::new();
    let mut current = geometry_index;
    for _ in 0..=parents.len() {
        chain.push(current);
        let Some(parent) = parents.get(current).copied().flatten() else {
            break;
        };
        current = parent;
    }
    if chain
        .last()
        .and_then(|index| parents.get(*index))
        .copied()
        .flatten()
        .is_some()
    {
        return None;
    }
    chain.reverse();
    let mut merged: Vec<EntityGeometryBone> = Vec::new();
    for index in chain {
        for child in assets.geometries().get(index)?.bones.iter() {
            if let Some(existing) = merged
                .iter_mut()
                .find(|bone| bone.name.eq_ignore_ascii_case(&child.name))
            {
                overlay_bone(existing, child);
            } else {
                merged.push(child.clone());
            }
        }
    }
    if merged.len() > MAX_RUNTIME_BONES_PER_RIG {
        return None;
    }
    merged
        .iter()
        .map(|bone| {
            let parent = bone.parent.as_ref().map(|name| {
                merged
                    .iter()
                    .position(|candidate| candidate.name.eq_ignore_ascii_case(name))
            });
            Some(RuntimeBone {
                parent: match parent {
                    Some(Some(index)) => Some(index),
                    Some(None) => return None,
                    None => None,
                },
                pivot: mirror_x(scalars(bone.pivot.as_ref())),
                rotation: scalars(bone.rotation.as_ref()),
            })
        })
        .collect()
}

fn overlay_bone(base: &mut EntityGeometryBone, child: &EntityGeometryBone) {
    if child.parent.is_some() {
        base.parent.clone_from(&child.parent);
    }
    if child.pivot.is_some() {
        base.pivot = child.pivot;
    }
    if child.rotation.is_some() {
        base.rotation = child.rotation;
    }
    if child.mirror.is_some() {
        base.mirror = child.mirror;
    }
    if child.inflate.is_some() {
        base.inflate = child.inflate;
    }
    if child.never_render.is_some() {
        base.never_render = child.never_render;
    }
    if child.reset.is_some() {
        base.reset = child.reset;
    }
    if !child.cubes.is_empty() {
        base.cubes.clone_from(&child.cubes);
    }
}

/// Maps authored geometry coordinates into the rig frame, whose X axis is mirrored.
fn mirror_x(point: [f32; 3]) -> [f32; 3] {
    [-point[0], point[1], point[2]]
}

fn scalars(values: Option<&[assets::EntityGeometryScalar; 3]>) -> [f32; 3] {
    values.map_or([0.0; 3], |values| values.map(|value| value.get()))
}

mod evaluation;
mod motion;
mod pose;
mod query;
mod tick;
use evaluation::{EngineSlots, Evaluator, MolangVariables, VariableLayout};
use motion::{MotionInput, MotionState};
use pose::{compose_pose, sample_clips};
pub(crate) use tick::ActorTickContext;
use tick::{advance_motion, evaluate_state};

#[cfg(test)]
mod tests;
