//! Attack-press actor picking, melee transactions and local arm swings.
//!
//! One attack per press; a held button never re-attacks. Swings obey the
//! half-swing guard shared with survival mining.

use bevy::{
    ecs::system::SystemParam,
    prelude::{Query, Res, ResMut, Resource, Window, With},
    window::PrimaryWindow,
};
use client_world::ActorSnapshot;
use protocol::{
    ActorUseAction, ActorUseRequest, BedrockSession, PlayerGameMode, PlayerInputMode, SwingSource,
};
use semantic_input::Action;

use crate::{
    interaction_authority::observe_block,
    local_player::InteractionOriginSnapshot,
    menu::MenuRuntime,
    mining::{creative_reach, protocol_input_mode, survival_reach, verified_selection},
    movement::{
        LocalMovementEffectTimeline, MiningEffects, MovementTicker, PhysicsCollisionRegistries,
    },
    runtime::{network::NetworkHandle, world::ClientWorld},
    semantic_controls::SemanticInputSnapshot,
    ui_runtime::UiRuntime,
};

/// Documented survival melee reach from the eye.
const SURVIVAL_ATTACK_REACH: f64 = 3.0;
/// Creative melee reach. Needs independent measurement.
const CREATIVE_ATTACK_REACH: f64 = 7.0;
/// Pick-box inflation and actor-versus-block bias. Needs independent measurement.
const ACTOR_PICK_RADIUS: f64 = 0.1;
/// Documented default swing length of 0.3 seconds.
const DEFAULT_SWING_TICKS: i32 = 6;
/// Ticks after an attack during which block use is suppressed. Needs independent measurement.
const ATTACK_BUILD_BLOCK_TICKS: u64 = 4;

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
}

impl SwingTracker {
    pub(crate) fn try_swing(&mut self, tick: u64, duration: i32) -> bool {
        let half = u64::try_from(duration / 2).unwrap_or(0);
        let allowed = self
            .last_swing_tick
            .is_none_or(|last| tick < last || tick - last >= half);
        if allowed {
            self.last_swing_tick = Some(tick);
        }
        allowed
    }
}

/// Attack-press state; `actor_in_front` vetoes mining behind a targeted actor.
#[derive(Resource, Debug, Default)]
pub(crate) struct MeleeRuntime {
    latched_press: bool,
    actor_in_front: bool,
    last_attack_tick: Option<u64>,
}

impl MeleeRuntime {
    pub(crate) const fn actor_in_front(&self) -> bool {
        self.actor_in_front
    }

    /// Whether a recent attack still suppresses block use on `tick`.
    pub(crate) fn blocks_use_at(&self, tick: u64) -> bool {
        self.last_attack_tick
            .is_some_and(|attack| tick >= attack && tick - attack < ATTACK_BUILD_BLOCK_TICKS)
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
}

/// Runs before the mining producers so they can defer to a targeted actor.
pub(crate) fn produce_melee(
    context: MeleeContext,
    mut runtime: ResMut<MeleeRuntime>,
    mut swings: ResMut<SwingTracker>,
    mut movement: ResMut<MovementTicker>,
) {
    runtime.actor_in_front = false;
    let focused =
        !context.menu.is_visible() && context.windows.single().is_ok_and(|window| window.focused);
    let game_mode = context.ui.player_game_mode();
    let attack_reach = match game_mode {
        Some(PlayerGameMode::Survival | PlayerGameMode::Adventure) => SURVIVAL_ATTACK_REACH,
        Some(PlayerGameMode::Creative) => CREATIVE_ATTACK_REACH,
        _ => 0.0,
    };
    let Some(input) = context.input.snapshot().filter(|_| {
        focused
            && attack_reach > 0.0
            && !context.ui.ui_focused()
            && movement.accepts_creative_mining()
    }) else {
        runtime.latched_press = false;
        return;
    };
    let attack = context.input.phase(Action::Attack);
    runtime.latched_press |= attack.pressed;
    if !runtime.latched_press && !attack.held {
        return;
    }
    let input_mode = protocol_input_mode(input.input_mode);
    let Some(crosshair) = resolve_crosshair(
        &context,
        input_mode,
        attack_reach,
        (input.authority_generation, input.frame_sequence),
        movement.interaction_authority_identity().1,
    ) else {
        runtime.latched_press = false;
        return;
    };
    runtime.actor_in_front = !matches!(crosshair, Crosshair::Block);
    if !runtime.latched_press {
        return;
    }
    let Some(sample) = movement.newest_unsent_sample() else {
        return;
    };
    let (tick, player_position) = (sample.tick, sample.position);
    runtime.latched_press = false;
    let Some(stream) = context.client_world.stream.as_ref() else {
        return;
    };
    let local_runtime_id = stream.local_player_runtime_id();
    let duration = swing_duration(context.effects.mining_effects());
    let mut swing = |source| {
        if swings.try_swing(tick, duration) {
            let _ = context
                .network
                .send_interaction_packet(protocol::swing_arm_packet(local_runtime_id, source));
        }
    };
    match crosshair {
        Crosshair::Actor(hit) => {
            swing(SwingSource::Attack);
            runtime.last_attack_tick = Some(tick);
            let Some(selection) = verified_selection(&context.ui) else {
                return;
            };
            // The item descriptor no longer reads session state.
            let session = BedrockSession { shield_item_id: 0 };
            if let Ok(packet) = protocol::use_actor_packet(
                ActorUseRequest {
                    actor_runtime_id: hit.runtime_id,
                    action: ActorUseAction::Attack,
                    selected_slot: selection.slot,
                    selected_item: selection.item,
                    player_position,
                    hit_position: hit.point,
                },
                &session,
            ) {
                let _ = context.network.send_interaction_packet(packet);
            }
        }
        Crosshair::Block => swing(SwingSource::Mine),
        Crosshair::Miss => {
            // Touch misses neither swing nor flag.
            if input_mode != PlayerInputMode::Touch {
                swing(SwingSource::Attack);
                movement.mark_missed_swing(tick);
            }
        }
    }
}

fn resolve_crosshair(
    context: &MeleeContext,
    input_mode: PlayerInputMode,
    attack_reach: f64,
    input_authority: (std::num::NonZeroU64, u64),
    position_authority_generation: u64,
) -> Option<Crosshair> {
    let ray = context.origin.outbound_ray()?;
    let stream = context.client_world.stream.as_ref()?;
    if ray.session_generation() != context.ui.session_id()
        || ray.session_generation() != stream.actor_session_id()
    {
        return None;
    }
    let reach = if context.ui.player_game_mode() == Some(PlayerGameMode::Creative) {
        creative_reach(input_mode)
    } else {
        survival_reach(input_mode)
    };
    let origin = ray.origin().to_array();
    let block_distance = verified_selection(&context.ui)
        .and_then(|selection| {
            observe_block(
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
            )
        })
        .map(|observed| {
            let hit = observed.target.position;
            let offset = observed.target.relative_hit;
            (0..3)
                .map(|axis| {
                    (f64::from(hit[axis]) + f64::from(offset[axis]) - f64::from(origin[axis]))
                        .powi(2)
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
    Some(classify(actor, block_distance, attack_reach))
}

#[cfg(test)]
mod tests;
