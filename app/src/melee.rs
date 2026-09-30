//! Attack-press actor picking, melee transactions and local arm swings.
//!
//! One attack per press; a held button never re-attacks. Swings obey the
//! half-swing guard shared with survival mining.

use bevy::{
    ecs::system::SystemParam,
    prelude::{Query, Real, Res, ResMut, Resource, Time, Window, With},
    window::PrimaryWindow,
};
use client_world::ActorSnapshot;
use protocol::{ActorUseAction, ActorUseRequest, BedrockSession, PlayerInputMode, SwingSource};
use semantic_input::Action;

use crate::{
    interaction_authority::{
        BlockRayUnavailable, MAX_PENDING_INTERACTION_FRAMES, observe_block_ray,
    },
    local_player::InteractionOriginSnapshot,
    menu::MenuRuntime,
    mining::{
        FrozenMiningSelection, creative_reach, hand_interaction_selection, protocol_input_mode,
        survival_reach,
    },
    movement::{
        LocalMovementEffectTimeline, MiningEffects, MovementTicker, PhysicsCollisionRegistries,
    },
    runtime::{network::NetworkHandle, world::ClientWorld},
    semantic_controls::SemanticInputSnapshot,
    ui_runtime::UiRuntime,
};

/// Pick-box inflation and actor-versus-block bias. Needs independent measurement.
const ACTOR_PICK_RADIUS: f64 = 0.1;
/// Documented default swing length of 0.3 seconds.
const DEFAULT_SWING_TICKS: i32 = 6;
/// Wall-clock time after an attack during which block use is suppressed. Needs
/// independent measurement.
const ATTACK_BUILD_BLOCK_MILLIS: u64 = 200;

/// Actors vanilla cannot pick: drops, orbs, projectiles and effect carriers.
const UNPICKABLE_ACTORS: &[&str] = &[
    "minecraft:item",
    "minecraft:xp_orb",
    "minecraft:arrow",
    "minecraft:thrown_trident",
    "minecraft:snowball",
    "minecraft:egg",
    "minecraft:ender_pearl",
    "minecraft:splash_potion",
    "minecraft:lingering_potion",
    "minecraft:xp_bottle",
    "minecraft:fireball",
    "minecraft:small_fireball",
    "minecraft:wither_skull",
    "minecraft:wither_skull_dangerous",
    "minecraft:dragon_fireball",
    "minecraft:wind_charge_projectile",
    "minecraft:breeze_wind_charge_projectile",
    "minecraft:fishing_hook",
    "minecraft:falling_block",
    "minecraft:lightning_bolt",
    "minecraft:area_effect_cloud",
    "minecraft:evocation_fang",
    "minecraft:eye_of_ender_signal",
    "minecraft:fireworks_rocket",
    "minecraft:llama_spit",
    "minecraft:shulker_bullet",
];

/// The nearest pickable actor along the crosshair ray.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct ActorHit {
    pub(crate) runtime_id: u64,
    pub(crate) distance: f64,
    pub(crate) point: [f32; 3],
}

/// What an attack press resolves to.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum Crosshair {
    Actor(ActorHit),
    Block,
    /// Nothing in reach, including an actor in front but beyond melee reach.
    Miss,
}

/// Nearest actor whose inflated box the ray enters within `reach`.
pub(crate) fn pick_actor<'a>(
    actors: impl Iterator<Item = &'a ActorSnapshot>,
    excluded_unique_id: Option<i64>,
    origin: [f32; 3],
    direction: [f32; 3],
    reach: f64,
) -> Option<ActorHit> {
    let origin = origin.map(f64::from);
    let length = direction
        .into_iter()
        .map(|axis| f64::from(axis).powi(2))
        .sum::<f64>()
        .sqrt();
    if !length.is_finite() || length == 0.0 {
        return None;
    }
    let direction = direction.map(|axis| f64::from(axis) / length);
    actors
        .filter(|actor| Some(actor.unique_id) != excluded_unique_id && pickable(actor))
        .filter_map(|actor| {
            let (min, max) = actor.bounding_box()?;
            let min = min.map(|axis| f64::from(axis) - ACTOR_PICK_RADIUS);
            let max = max.map(|axis| f64::from(axis) + ACTOR_PICK_RADIUS);
            let distance = ray_box_entry(origin, direction, min, max)?;
            (distance <= reach).then(|| ActorHit {
                runtime_id: actor.runtime_id,
                distance,
                point: [0, 1, 2].map(|axis| (origin[axis] + direction[axis] * distance) as f32),
            })
        })
        .min_by(|left, right| left.distance.total_cmp(&right.distance))
}

/// Drops, orbs, projectiles and effect carriers never block a placement either.
pub(crate) fn obstructs_placement(actor: &ActorSnapshot) -> bool {
    pickable(actor)
}

fn pickable(actor: &ActorSnapshot) -> bool {
    match &actor.kind {
        protocol::ActorKind::Player { .. } => true,
        protocol::ActorKind::Entity { identifier } => {
            !UNPICKABLE_ACTORS.contains(&identifier.as_ref())
        }
    }
}

/// Distance along a unit `direction` at which the ray enters the box; zero from inside.
fn ray_box_entry(
    origin: [f64; 3],
    direction: [f64; 3],
    min: [f64; 3],
    max: [f64; 3],
) -> Option<f64> {
    let mut near = 0.0_f64;
    let mut far = f64::INFINITY;
    for axis in 0..3 {
        if direction[axis].abs() < f64::EPSILON {
            if origin[axis] < min[axis] || origin[axis] > max[axis] {
                return None;
            }
            continue;
        }
        let first = (min[axis] - origin[axis]) / direction[axis];
        let second = (max[axis] - origin[axis]) / direction[axis];
        near = near.max(first.min(second));
        far = far.min(first.max(second));
        if near > far {
            return None;
        }
    }
    Some(near)
}

/// Resolves the press target; an actor wins only when clearly in front of the block.
pub(crate) fn classify(
    actor: Option<ActorHit>,
    block_distance: Option<f64>,
    attack_reach: f64,
) -> Crosshair {
    let limit = block_distance.unwrap_or(f64::INFINITY);
    match actor {
        Some(hit) if hit.distance + ACTOR_PICK_RADIUS < limit.min(attack_reach) => {
            Crosshair::Actor(hit)
        }
        Some(hit) if hit.distance + ACTOR_PICK_RADIUS < limit => Crosshair::Miss,
        _ if block_distance.is_some() => Crosshair::Block,
        _ => Crosshair::Miss,
    }
}

/// Swing length in ticks under the current effects. Adjustments need independent measurement.
pub(crate) fn swing_duration(effects: MiningEffects) -> i32 {
    let level =
        |amplifier: Option<i32>| amplifier.filter(|value| *value >= 0).map(|value| value + 1);
    let haste = level(effects.haste).max(level(effects.conduit_power));
    let duration = match (haste, level(effects.mining_fatigue)) {
        (Some(haste), _) => DEFAULT_SWING_TICKS - haste,
        (None, Some(fatigue)) => DEFAULT_SWING_TICKS.saturating_add(fatigue.saturating_mul(2)),
        (None, None) => DEFAULT_SWING_TICKS,
    };
    duration.max(1)
}

/// The local arm-swing guard: a new swing starts once half the current one elapsed.
#[derive(Resource, Debug, Default)]
pub(crate) struct SwingTracker {
    last_swing_tick: Option<u64>,
    /// A swing started since the local rig last took it.
    started: bool,
}

impl SwingTracker {
    pub(crate) fn try_swing(&mut self, tick: u64, duration: i32) -> bool {
        let half = u64::try_from(duration / 2).unwrap_or(0);
        let allowed = self
            .last_swing_tick
            .is_none_or(|last| tick < last || tick - last >= half);
        if allowed {
            self.last_swing_tick = Some(tick);
            self.started = true;
        }
        allowed
    }

    /// Whether a swing started since the last call; the local rig plays it.
    pub(crate) fn take_started(&mut self) -> bool {
        std::mem::take(&mut self.started)
    }
}

/// The unsent tick and local state one attack press resolves against.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct PressContext {
    pub(crate) tick: u64,
    pub(crate) player_position: [f32; 3],
    pub(crate) input_mode: PlayerInputMode,
    pub(crate) local_runtime_id: u64,
    pub(crate) selection: Option<FrozenMiningSelection>,
    pub(crate) swing_duration: i32,
    pub(crate) now_millis: u64,
}

/// Standalone packets in send order, plus whether the tick reports a missed swing.
#[derive(Debug, Default)]
pub(crate) struct MeleeOutcome {
    pub(crate) packets: Vec<protocol::Packet>,
    pub(crate) missed_swing: bool,
}

/// Attack-press state; `actor_in_front` vetoes mining behind a targeted actor.
#[derive(Resource, Debug, Default)]
pub(crate) struct MeleeRuntime {
    latched_press: bool,
    actor_in_front: bool,
    last_attack_millis: Option<u64>,
    position_authority: Option<(u64, u64)>,
    /// Input frame at which a latched press first waited on block evidence.
    deferred_since: Option<u64>,
}

impl MeleeRuntime {
    pub(crate) const fn actor_in_front(&self) -> bool {
        self.actor_in_front
    }

    /// Whether a recent attack still suppresses block use.
    pub(crate) fn blocks_use_at(&self, now_millis: u64) -> bool {
        self.last_attack_millis.is_some_and(|attack| {
            now_millis >= attack && now_millis - attack < ATTACK_BUILD_BLOCK_MILLIS
        })
    }

    /// Drops a latched press when the session or position authority changes.
    pub(crate) fn synchronize(&mut self, authority: (u64, u64)) {
        if self
            .position_authority
            .is_some_and(|previous| previous != authority)
        {
            self.latched_press = false;
            self.last_attack_millis = None;
        }
        self.position_authority = Some(authority);
    }

    /// Records this frame's attack input; returns whether a press is waiting.
    pub(crate) fn observe_input(&mut self, pressed: bool, held: bool) -> bool {
        self.latched_press |= pressed;
        self.latched_press || held
    }

    fn observe_crosshair(&mut self, crosshair: Crosshair) {
        self.actor_in_front = !matches!(crosshair, Crosshair::Block);
    }

    pub(crate) fn cancel(&mut self) {
        self.latched_press = false;
        self.actor_in_front = false;
        self.deferred_since = None;
    }

    /// Holds a latched press while block evidence is unavailable, for a bounded number of frames.
    pub(crate) fn defer(&mut self, input_frame: u64) {
        if !self.latched_press {
            return;
        }
        let since = *self.deferred_since.get_or_insert(input_frame);
        if input_frame.saturating_sub(since) > MAX_PENDING_INTERACTION_FRAMES {
            self.cancel();
        }
    }

    /// Resolves at most one latched press into packets; a held button never re-attacks.
    pub(crate) fn resolve(
        &mut self,
        crosshair: Crosshair,
        press: &PressContext,
        swings: &mut SwingTracker,
    ) -> MeleeOutcome {
        self.observe_crosshair(crosshair);
        self.deferred_since = None;
        let mut outcome = MeleeOutcome::default();
        if !std::mem::take(&mut self.latched_press) {
            return outcome;
        }
        let mut swing = |outcome: &mut MeleeOutcome, source| {
            if swings.try_swing(press.tick, press.swing_duration) {
                outcome
                    .packets
                    .push(protocol::swing_arm_packet(press.local_runtime_id, source));
            }
        };
        match crosshair {
            Crosshair::Actor(hit) => {
                swing(&mut outcome, SwingSource::Attack);
                self.last_attack_millis = Some(press.now_millis);
                let Some(selection) = press.selection.clone() else {
                    return outcome;
                };
                // The item descriptor no longer reads session state.
                let session = BedrockSession { shield_item_id: 0 };
                if let Ok(packet) = protocol::use_actor_packet(
                    ActorUseRequest {
                        actor_runtime_id: hit.runtime_id,
                        action: ActorUseAction::Attack,
                        selected_slot: selection.slot,
                        selected_item: selection.item,
                        player_position: press.player_position,
                        hit_position: hit.point,
                    },
                    &session,
                ) {
                    outcome.packets.push(packet);
                }
            }
            Crosshair::Block => swing(&mut outcome, SwingSource::Mine),
            Crosshair::Miss => {
                // Touch skips the early swing on a miss but still reports it.
                if press.input_mode != PlayerInputMode::Touch {
                    swing(&mut outcome, SwingSource::Attack);
                }
                outcome.missed_swing = true;
            }
        }
        outcome
    }
}

#[derive(SystemParam)]
pub(crate) struct MeleeContext<'w, 's> {
    input: Res<'w, SemanticInputSnapshot>,
    origin: Res<'w, InteractionOriginSnapshot>,
    ui: Res<'w, UiRuntime>,
    menu: Res<'w, MenuRuntime>,
    windows: Query<'w, 's, &'static Window, With<PrimaryWindow>>,
    client_world: Res<'w, ClientWorld>,
    collisions: Res<'w, PhysicsCollisionRegistries>,
    effects: Res<'w, LocalMovementEffectTimeline>,
    network: Res<'w, NetworkHandle>,
    time: Res<'w, Time<Real>>,
}

/// Runs before the mining producers so they can defer to a targeted actor.
pub(crate) fn produce_melee(
    context: MeleeContext,
    mut runtime: ResMut<MeleeRuntime>,
    mut swings: ResMut<SwingTracker>,
    mut movement: ResMut<MovementTicker>,
) {
    runtime.synchronize(movement.interaction_authority_identity());
    let focused =
        !context.menu.is_visible() && context.windows.single().is_ok_and(|window| window.focused);
    let caps = context.ui.game_mode_capabilities();
    let attack_reach = caps.map_or(0.0, |caps| caps.attack_reach);
    let Some(input) = context.input.snapshot().filter(|_| {
        focused
            && caps.is_some_and(|caps| caps.can_attack)
            && !context.ui.ui_focused()
            && movement.accepts_creative_mining()
    }) else {
        runtime.cancel();
        return;
    };
    let attack = context.input.phase(Action::Attack);
    if !runtime.observe_input(attack.pressed, attack.held) {
        runtime.cancel();
        return;
    }
    let input_mode = protocol_input_mode(input.input_mode);
    let (Some(observation), Some(stream)) = (
        resolve_crosshair(
            &context,
            input_mode,
            attack_reach,
            caps.is_some_and(|caps| caps.creative_reach),
            (input.authority_generation, input.frame_sequence),
            movement.interaction_authority_identity().1,
        ),
        context.client_world.stream.as_ref(),
    ) else {
        runtime.cancel();
        return;
    };
    // Unverified occlusion never admits an attack; the press waits for fresh evidence.
    let Ok(crosshair) = observation else {
        runtime.defer(input.frame_sequence);
        return;
    };
    runtime.observe_crosshair(crosshair);
    // Frames between physics ticks have no unsent tick; the press waits for one.
    let Some(sample) = movement.newest_unsent_sample() else {
        return;
    };
    let press = PressContext {
        tick: sample.tick,
        player_position: sample.position,
        input_mode,
        local_runtime_id: stream.local_player_runtime_id(),
        selection: hand_interaction_selection(&context.ui),
        swing_duration: swing_duration(context.effects.mining_effects()),
        now_millis: u64::try_from(context.time.elapsed().as_millis()).unwrap_or(u64::MAX),
    };
    let outcome = runtime.resolve(crosshair, &press, &mut swings);
    for packet in outcome.packets {
        let _ = context.network.send_inventory_packet(packet);
    }
    if outcome.missed_swing {
        movement.mark_missed_swing(sample.tick);
    }
}

fn resolve_crosshair(
    context: &MeleeContext,
    input_mode: PlayerInputMode,
    attack_reach: f64,
    creative_pick_reach: bool,
    input_authority: (std::num::NonZeroU64, u64),
    position_authority_generation: u64,
) -> Option<Result<Crosshair, BlockRayUnavailable>> {
    let ray = context.origin.outbound_ray()?;
    let stream = context.client_world.stream.as_ref()?;
    if ray.session_generation() != context.ui.session_id()
        || ray.session_generation() != stream.actor_session_id()
    {
        return None;
    }
    let reach = if creative_pick_reach {
        creative_reach(input_mode)
    } else {
        survival_reach(input_mode)
    };
    let origin = ray.origin().to_array();
    let Some(selection) = hand_interaction_selection(&context.ui) else {
        return Some(Err(BlockRayUnavailable));
    };
    let observed = match observe_block_ray(
        &context.origin,
        &context.ui,
        &context.client_world,
        &context.collisions,
        selection,
        (
            input_mode,
            reach,
            input_authority,
            position_authority_generation,
        ),
    ) {
        Ok(observed) => observed,
        Err(unavailable) => return Some(Err(unavailable)),
    };
    let block_distance = observed.map(|observed| {
        let hit = observed.target.position;
        let offset = observed.target.relative_hit;
        (0..3)
            .map(|axis| {
                (f64::from(hit[axis]) + f64::from(offset[axis]) - f64::from(origin[axis])).powi(2)
            })
            .sum::<f64>()
            .sqrt()
    });
    let actor = pick_actor(
        stream.remote_actors(),
        context.ui.gameplay_hud().mount_unique_id(),
        origin,
        ray.direction().to_array(),
        reach,
    );
    Some(Ok(classify(actor, block_distance, attack_reach)))
}

#[cfg(test)]
mod tests;
