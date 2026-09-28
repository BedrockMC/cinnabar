//! Survival hold-to-mine: one destroy state-machine step per completed physics tick.
//!
//! Completion is only predicted from the provisional destroy table; inbound block
//! updates remain the sole block-change authority. Unknown blocks keep cracking
//! without a prediction so the server decides when they break.

use bevy::{
    ecs::system::SystemParam,
    prelude::{Query, Real, Res, ResMut, Resource, Time, Window, With},
    window::PrimaryWindow,
};
use protocol::{
    BlockAction, BlockActionKind, BlockActions, BlockItemInteraction, BlockUseRequest,
    PlayerAuthInputInteractions,
};
use semantic_input::Action;
use sim::{BlockDestroyInfo, DestroyConditions, HeldTool, PaletteWorld};

use crate::{
    game_mode_capabilities::GameModeCapabilities,
    interaction_authority::{observe_block, within_pick_range},
    local_player::InteractionOriginSnapshot,
    melee::{MeleeRuntime, SwingTracker, swing_duration},
    menu::MenuRuntime,
    mining::{
        FrozenMiningSelection, hand_interaction_selection, protocol_input_mode, survival_reach,
    },
    movement::{LocalMovementEffectTimeline, MovementTicker, PhysicsCollisionRegistries},
    runtime::{network::NetworkHandle, world::ClientWorld},
    semantic_controls::SemanticInputSnapshot,
    ui_runtime::UiRuntime,
};

/// Ticks between a completed destroy and the next start. Needs independent measurement.
pub(crate) const DESTROY_DELAY_TICKS: u8 = 5;
/// Progress at which a destroy completes, absorbing float accumulation. Needs
/// independent measurement.
const COMPLETION_THRESHOLD: f64 = 0.99999;
/// Bedrock enchantment ids.
const AQUA_AFFINITY_ENCHANTMENT_ID: i16 = 8;
const EFFICIENCY_ENCHANTMENT_ID: i16 = 15;
/// How long a predicted break suppresses restarting on the unchanged block.
const PREDICTED_BREAK_HOLD_TICKS: u8 = 20;

/// Which side StartGame's negotiation makes authoritative for block destruction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BlockBreakingAuthority {
    /// Progress travels as per-tick block actions; completion is only predicted.
    Server,
    /// Completion travels as an item-use destroy transaction.
    Client,
}

impl BlockBreakingAuthority {
    const fn from_negotiation(server_authoritative: bool) -> Self {
        if server_authoritative {
            Self::Server
        } else {
            Self::Client
        }
    }
}

/// The block under the crosshair and everything its destroy rate depends on.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct DestroyTarget {
    pub(crate) position: [i32; 3],
    pub(crate) face: u8,
    pub(crate) runtime_id: u32,
    pub(crate) relative_hit: [f32; 3],
    pub(crate) block: Option<BlockDestroyInfo>,
    /// Everything except `on_ground`, which is taken from each stepped tick.
    pub(crate) conditions: DestroyConditions,
    pub(crate) selection: FrozenMiningSelection,
    /// The held tool's wear from a non-instant destroy, when it is predictable.
    pub(crate) wear: Option<ToolWear>,
}

/// Held-tool damage before a destroy and the damage one destroy adds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ToolWear {
    pub(crate) current_damage: i32,
    pub(crate) break_damage: i32,
}

impl DestroyTarget {
    fn rate(&self, on_ground: bool) -> f64 {
        let conditions = DestroyConditions {
            on_ground,
            ..self.conditions
        };
        self.block
            .as_ref()
            .and_then(|block| sim::destroy_progress_per_tick(block, &conditions))
            .map_or(0.0, f64::from)
    }
}

/// Destroy actions for one tick, plus the client-authoritative completion if any.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct SurvivalTickPayload {
    pub(crate) actions: BlockActions,
    pub(crate) destroy: Option<DestroyTarget>,
    /// Holding attack on a block attempts a mining swing every tick.
    pub(crate) swing: bool,
    /// `(slot, predicted damage, stack network id)` of a worn held tool.
    pub(crate) wear: Option<(u8, i32, i32)>,
    /// The request carrying `wear`, once a request id is allocated.
    pub(crate) mine_block: Option<protocol::MineBlockRequest>,
}

impl SurvivalTickPayload {
    pub(crate) fn is_empty(&self) -> bool {
        self.actions.is_empty() && self.destroy.is_none() && self.mine_block.is_none()
    }

    /// Faces, percents, hit offsets and slots are bounded at their sources, so
    /// the carrier's own validation cannot reject the result.
    pub(crate) fn into_interactions(
        self,
        player_position: [f32; 3],
    ) -> (
        PlayerAuthInputInteractions,
        Option<protocol::MineBlockRequest>,
    ) {
        let block_interaction = self.destroy.map(|target| {
            BlockItemInteraction::Destroy(BlockUseRequest {
                block_position: target.position,
                face: target.face,
                selected_slot: target.selection.slot,
                selected_item: target.selection.item,
                player_position,
                relative_hit: target.relative_hit,
                block_runtime_id: u64::from(target.runtime_id),
            })
        });
        let interactions = PlayerAuthInputInteractions {
            block_actions: self.actions,
            block_interaction,
        };
        (interactions, self.mine_block)
    }

    fn push(&mut self, kind: BlockActionKind, position: [i32; 3], face: u8) {
        // A tick emits at most four actions; the bounded list holds eight.
        let _ = self.actions.push(BlockAction {
            kind,
            position,
            face,
        });
    }
}

/// Attack input for one tick.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum DestroyInput<'a> {
    Released,
    /// Held, aimed at this block or at nothing destroyable.
    Held(Option<&'a DestroyTarget>),
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct Destroying {
    position: [i32; 3],
    face: u8,
    progress: f64,
}

impl Destroying {
    fn abort_percent(&self) -> u8 {
        (self.progress * 100.0).clamp(0.0, 100.0) as u8
    }
}

/// Per-tick survival destroy sequencing.
///
/// Server authority: start, then silence while cracking, a continue on a new
/// block, and continue plus predict on completion; aborts carry progress.
/// Client authority: start, a crack every tick, and a stop plus item-use
/// destroy transaction on completion.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct DestroyMachine {
    destroying: Option<Destroying>,
    delay: u8,
    pending_abort: Option<([i32; 3], u8)>,
    /// A predicted break whose block update has not arrived yet.
    predicted_break: Option<([i32; 3], u32, u8)>,
}

impl DestroyMachine {
    /// Forgets unsent progress; an in-flight destroy is aborted on the next step.
    pub(crate) fn interrupt(&mut self) {
        if let Some(destroying) = self.destroying.take() {
            self.pending_abort = Some((destroying.position, destroying.abort_percent()));
        }
    }

    pub(crate) fn step(
        &mut self,
        input: DestroyInput<'_>,
        on_ground: bool,
        authority: BlockBreakingAuthority,
    ) -> SurvivalTickPayload {
        let mut payload = SurvivalTickPayload::default();
        if let Some((position, percent)) = self.pending_abort.take() {
            payload.push(BlockActionKind::AbortDestroy, position, percent);
        }
        if let Some((_, _, remaining)) = &mut self.predicted_break {
            *remaining = remaining.saturating_sub(1);
        }
        self.predicted_break = self
            .predicted_break
            .filter(|(_, _, remaining)| *remaining > 0);
        let delayed = self.delay > 0;
        self.delay = self.delay.saturating_sub(1);
        let target = match input {
            DestroyInput::Held(Some(target)) => {
                payload.swing = true;
                target
            }
            DestroyInput::Released | DestroyInput::Held(None) => {
                if let Some(destroying) = self.destroying.take() {
                    payload.push(
                        BlockActionKind::AbortDestroy,
                        destroying.position,
                        destroying.abort_percent(),
                    );
                }
                return payload;
            }
        };
        // Locally the block is already gone; wait for its update or the hold.
        if delayed || self.awaiting_update(target) {
            return payload;
        }
        match self.destroying {
            None => {
                payload.push(BlockActionKind::StartDestroy, target.position, target.face);
                self.destroying = Some(Destroying {
                    position: target.position,
                    face: target.face,
                    progress: 0.0,
                });
                // A rate at the threshold breaks on the start tick, then delays.
                if target.rate(on_ground) >= COMPLETION_THRESHOLD {
                    self.complete(&mut payload, target, authority, false);
                    self.delay = DESTROY_DELAY_TICKS;
                } else if authority == BlockBreakingAuthority::Client {
                    payload.push(BlockActionKind::CrackBlock, target.position, target.face);
                }
            }
            Some(destroying) if destroying.position == target.position => {
                let rate = target.rate(on_ground);
                let progress = destroying.progress + rate;
                if progress >= COMPLETION_THRESHOLD {
                    self.complete(&mut payload, target, authority, true);
                    self.delay = if rate < 1.0 { DESTROY_DELAY_TICKS } else { 0 };
                } else {
                    self.destroying = Some(Destroying {
                        progress,
                        ..destroying
                    });
                    if authority == BlockBreakingAuthority::Client {
                        payload.push(BlockActionKind::CrackBlock, target.position, target.face);
                    }
                }
            }
            Some(destroying) => {
                match authority {
                    BlockBreakingAuthority::Server => payload.push(
                        BlockActionKind::ContinueDestroy,
                        target.position,
                        target.face,
                    ),
                    BlockBreakingAuthority::Client => {
                        payload.push(
                            BlockActionKind::AbortDestroy,
                            destroying.position,
                            destroying.abort_percent(),
                        );
                        payload.push(BlockActionKind::StartDestroy, target.position, target.face);
                        payload.push(BlockActionKind::CrackBlock, target.position, target.face);
                    }
                }
                self.destroying = Some(Destroying {
                    position: target.position,
                    face: target.face,
                    progress: 0.0,
                });
            }
        }
        payload
    }

    fn awaiting_update(&self, target: &DestroyTarget) -> bool {
        self.predicted_break
            .is_some_and(|(position, runtime_id, _)| {
                position == target.position && runtime_id == target.runtime_id
            })
    }

    /// The destroy stays active on the broken block, so the next target continues it.
    /// A server-authoritative destroy of a block with hardness wears the tool.
    fn complete(
        &mut self,
        payload: &mut SurvivalTickPayload,
        target: &DestroyTarget,
        authority: BlockBreakingAuthority,
        continued: bool,
    ) {
        match authority {
            BlockBreakingAuthority::Server => {
                let worn = target.block.is_some_and(|block| block.hardness > 0.0);
                payload.wear = target.wear.filter(|_| worn).map(|wear| {
                    (
                        target.selection.slot,
                        wear.current_damage.saturating_add(wear.break_damage),
                        target.selection.item.stack_network_id(),
                    )
                });
                if continued {
                    payload.push(
                        BlockActionKind::ContinueDestroy,
                        target.position,
                        target.face,
                    );
                }
                payload.push(
                    BlockActionKind::PredictDestroy,
                    target.position,
                    target.face,
                );
            }
            BlockBreakingAuthority::Client => {
                payload.push(BlockActionKind::StopDestroy, [0; 3], 0);
                payload.destroy = Some(target.clone());
            }
        }
        self.destroying = Some(Destroying {
            position: target.position,
            face: target.face,
            progress: 0.0,
        });
        self.predicted_break = Some((
            target.position,
            target.runtime_id,
            PREDICTED_BREAK_HOLD_TICKS,
        ));
    }
}

/// Wall-clock spacing between block-diagnostic lines while an attack is held.
const BLOCKED_MINING_LOG_THROTTLE_MILLIS: u64 = 2000;

/// Survival destroy sequencing bound to the current position authority.
#[derive(Resource, Debug, Default)]
pub(crate) struct SurvivalMiningRuntime {
    machine: DestroyMachine,
    last_stepped_tick: Option<u64>,
    latched_press: bool,
    position_authority: Option<(u64, u64)>,
    last_blocked_log_millis: Option<u64>,
}

impl SurvivalMiningRuntime {
    /// Steps every unsent tick once, attaching nonempty payloads to their
    /// samples, and returns the mining request ids no tick carried.
    pub(crate) fn step_ticks(
        &mut self,
        ticker: &mut MovementTicker,
        input: DestroyInput<'_>,
        authority: BlockBreakingAuthority,
        mut swing: impl FnMut(u64),
        mut request_id: impl FnMut(u8, i32) -> Option<i32>,
    ) -> Vec<i32> {
        let mut unsent = Vec::new();
        let identity = ticker.interaction_authority_identity();
        if let Some((session, _)) = self
            .position_authority
            .filter(|previous| *previous != identity)
        {
            // Reanchors clear the outbox and may rewind tick numbers.
            self.last_stepped_tick = None;
            if session == identity.0 {
                self.machine.interrupt();
            } else {
                self.machine = DestroyMachine::default();
            }
        }
        self.position_authority = Some(identity);
        let ticks = ticker.unstepped_interaction_ticks(self.last_stepped_tick);
        let Some(&(newest, _)) = ticks.last() else {
            return unsent;
        };
        self.last_stepped_tick = Some(newest);
        if !ticker.accepts_creative_mining() {
            // Withheld ticks never reach the server, so neither may their actions.
            self.machine.interrupt();
            return unsent;
        }
        for (tick, on_ground) in ticks {
            let mut payload = self.machine.step(input, on_ground, authority);
            payload.mine_block = payload
                .wear
                .filter(|&(slot, damage, stack_network_id)| {
                    slot <= 8 && damage >= 0 && stack_network_id > 0
                })
                .and_then(|(slot, damage, stack_network_id)| {
                    protocol::MineBlockRequest::new(
                        request_id(slot, damage)?,
                        slot,
                        damage,
                        stack_network_id,
                    )
                    .ok()
                });
            self.latched_press = false;
            if payload.swing {
                swing(tick);
            }
            let mine_block = payload
                .mine_block
                .as_ref()
                .map(|request| request.request_id());
            if !payload.is_empty() && !ticker.attach_survival_mining(tick, payload) {
                // A tick that cannot carry its actions desynchronizes the server's view.
                self.machine.interrupt();
                unsent.extend(mine_block);
            }
        }
        unsent
    }

    /// Emits one throttled line naming the gate that blocked a held-attack break.
    fn log_blocked_mining(
        &mut self,
        now_millis: u64,
        reason: &'static str,
        caps: Option<GameModeCapabilities>,
        authority: Option<BlockBreakingAuthority>,
    ) {
        let due = self.last_blocked_log_millis.is_none_or(|last| {
            now_millis.saturating_sub(last) >= BLOCKED_MINING_LOG_THROTTLE_MILLIS
        });
        if !due {
            return;
        }
        self.last_blocked_log_millis = Some(now_millis);
        bevy::log::debug!(
            target: "bedrock_client::survival_mining",
            reason,
            game_mode_known = caps.is_some(),
            can_edit = caps.is_some_and(|caps| caps.can_edit),
            instant_break = caps.is_some_and(|caps| caps.instant_break),
            authority = ?authority,
            "held attack produced no block break",
        );
    }
}

#[derive(SystemParam)]
pub(crate) struct SurvivalMiningContext<'w, 's> {
    input: Res<'w, SemanticInputSnapshot>,
    origin: Res<'w, InteractionOriginSnapshot>,
    ui: ResMut<'w, UiRuntime>,
    menu: Res<'w, MenuRuntime>,
    windows: Query<'w, 's, &'static Window, With<PrimaryWindow>>,
    client_world: Res<'w, ClientWorld>,
    collisions: Res<'w, PhysicsCollisionRegistries>,
    effects: Res<'w, LocalMovementEffectTimeline>,
    melee: Res<'w, MeleeRuntime>,
    network: Res<'w, NetworkHandle>,
    time: Res<'w, Time<Real>>,
}

/// Runs after committed world publication and before the movement flush.
pub(crate) fn produce_survival_mining(
    mut context: SurvivalMiningContext,
    mut runtime: ResMut<SurvivalMiningRuntime>,
    mut swings: ResMut<SwingTracker>,
    mut movement: ResMut<MovementTicker>,
) {
    // Wire sequencing only, defaulted to server-authoritative when the server
    // never negotiated it. This decides HOW a break travels, never WHETHER one
    // may happen; the capability gate below owns that.
    let authority = context
        .ui
        .server_authoritative_block_breaking()
        .map(BlockBreakingAuthority::from_negotiation);
    let caps = context.ui.game_mode_capabilities();
    let focused =
        !context.menu.is_visible() && context.windows.single().is_ok_and(|window| window.focused);
    let attack = context.input.phase(Action::Attack);
    let snapshot = context.input.snapshot();
    let snapshot_present = snapshot.is_some();
    let actor_in_front = context.melee.actor_in_front();
    let active = survival_mining_active(caps, focused, snapshot_present);
    let target = match snapshot.filter(|_| active) {
        Some(input) => {
            runtime.latched_press |= attack.pressed;
            (attack.held || runtime.latched_press).then(|| {
                // An actor in front owns the press; the block behind it is not a target.
                (!actor_in_front)
                    .then(|| {
                        observe_destroy_target(
                            &context,
                            input.input_mode,
                            (input.authority_generation, input.frame_sequence),
                            movement.interaction_authority_identity().1,
                        )
                    })
                    .flatten()
            })
        }
        None => {
            runtime.latched_press = false;
            None
        }
    };
    if (attack.pressed || attack.held)
        && let Some(reason) = blocked_mining_reason(
            caps,
            focused,
            snapshot_present,
            actor_in_front,
            matches!(target, Some(Some(_))),
        )
    {
        let now = u64::try_from(context.time.elapsed().as_millis()).unwrap_or(u64::MAX);
        runtime.log_blocked_mining(now, reason, caps, authority);
    }
    let input = target.as_ref().map_or(DestroyInput::Released, |target| {
        DestroyInput::Held(target.as_ref())
    });
    let duration = swing_duration(context.effects.mining_effects());
    let local_runtime_id = context
        .client_world
        .stream
        .as_ref()
        .map(|stream| stream.local_player_runtime_id());
    let network = &context.network;
    let ui = &mut context.ui;
    let unsent = runtime.step_ticks(
        &mut movement,
        input,
        authority.unwrap_or(BlockBreakingAuthority::Server),
        |tick| {
            if let Some(local_runtime_id) = local_runtime_id
                && swings.try_swing(tick, duration)
            {
                let _ = network.send_inventory_packet(protocol::swing_arm_packet(
                    local_runtime_id,
                    protocol::SwingSource::Mine,
                ));
            }
        },
        |slot, damage| ui.begin_mining_request(slot, damage),
    );
    for request_id in unsent {
        ui.cancel_mining_request(request_id);
    }
}

/// Whether held-mining should look for a destroy target this frame. The
/// block-breaking wire mode is deliberately not an input here: it sequences a
/// break, it never decides whether one may happen.
fn survival_mining_active(
    caps: Option<GameModeCapabilities>,
    focused: bool,
    snapshot_present: bool,
) -> bool {
    focused && snapshot_present && caps.is_some_and(|caps| caps.uses_survival_mining())
}

/// Why a held attack yielded no destroy target, for the throttled diagnostic.
fn blocked_mining_reason(
    caps: Option<GameModeCapabilities>,
    focused: bool,
    snapshot_present: bool,
    actor_in_front: bool,
    target_found: bool,
) -> Option<&'static str> {
    match caps {
        None => Some("game mode unknown"),
        Some(caps) if !caps.can_edit => Some("can_edit=false for this game mode"),
        Some(caps) if caps.instant_break => Some("instant-break mode uses the creative path"),
        _ if !focused => Some("window or menu not focused"),
        _ if !snapshot_present => Some("no input snapshot yet"),
        _ if actor_in_front => Some("an actor in front owns the press"),
        _ if !target_found => Some("no breakable block in reach"),
        _ => None,
    }
}

fn observe_destroy_target(
    context: &SurvivalMiningContext,
    input_mode: semantic_input::InputMode,
    input_authority: (std::num::NonZeroU64, u64),
    position_authority_generation: u64,
) -> Option<DestroyTarget> {
    let ui = &context.ui;
    // The capability gate in the producer already confirmed this mode edits and
    // is not the instant-break path; here only an open UI blocks the pick.
    if ui.ui_focused() {
        return None;
    }
    let selection = hand_interaction_selection(ui)?;
    let input_mode = protocol_input_mode(input_mode);
    let observed = observe_block(
        &context.origin,
        ui,
        &context.client_world,
        &context.collisions,
        selection,
        (
            input_mode,
            survival_reach(input_mode),
            input_authority,
            position_authority_generation,
        ),
    )?;
    if !within_pick_range(&observed) {
        return None;
    }
    let stream = context.client_world.stream.as_ref()?;
    let mode = stream.network_id_mode();
    let block = context
        .collisions
        .block_identifier(mode, observed.target.runtime_id)
        .and_then(sim::block_destroy_info);
    let item = &observed.selection.item;
    let tool = (item.network_id() != 0)
        .then(|| {
            ui.inventory_ledger()
                .negotiated_item_entry(item.network_id())
        })
        .flatten()
        .and_then(|entry| HeldTool::from_identifier(entry.identifier.as_ref()));
    let world = PaletteWorld::new(
        stream.collision_store(),
        context.collisions.registry(mode),
        stream.current_dimension(),
    );
    let effects = context.effects.mining_effects();
    let helmet = ui.gameplay_hud().armor().map(|armor| &armor.helmet);
    let wear = tool.and_then(|tool| {
        (item.stack_network_id() > 0).then(|| ToolWear {
            // Outstanding and corrected predictions outrank the stack's own tag.
            current_damage: ui
                .inventory_ledger()
                .predicted_slot_damage(observed.selection.slot)
                .or_else(|| {
                    protocol::item_extra_damage(item.extra_data())
                        .and_then(|damage| i32::try_from(damage).ok())
                })
                .unwrap_or(0),
            break_damage: tool_break_damage(tool.kind),
        })
    });
    Some(DestroyTarget {
        position: observed.target.position,
        face: observed.target.face,
        runtime_id: observed.target.runtime_id,
        relative_hit: observed.target.relative_hit,
        block,
        conditions: DestroyConditions {
            tool,
            efficiency_level: protocol::item_enchantment_level(
                item.extra_data(),
                EFFICIENCY_ENCHANTMENT_ID,
            )
            .unwrap_or(0),
            haste_amplifier: effects.haste,
            conduit_power_amplifier: effects.conduit_power,
            mining_fatigue_amplifier: effects.mining_fatigue,
            on_ground: true,
            // Local simulation has no flight and never reports starting to fly.
            flying: false,
            riding: ui.gameplay_hud().mount_unique_id().is_some(),
            eyes_in_water: eyes_in_water(&world, observed.ray.origin),
            // Unknown armor reads as absent, which only slows prediction.
            aqua_affinity: helmet.is_some_and(|helmet| {
                protocol::item_enchantment_level(&helmet.extra_data, AQUA_AFFINITY_ENCHANTMENT_ID)
                    .is_some_and(|level| level > 0)
            }),
        },
        selection: observed.selection,
        wear,
    })
}

/// Durability one destroy costs, per dragonfly's `item/*.go` durability info (MIT).
const fn tool_break_damage(kind: sim::ToolKind) -> i32 {
    match kind {
        sim::ToolKind::Sword => 2,
        _ => 1,
    }
}

/// Unreadable eye blocks count as submerged, which only slows prediction.
fn eyes_in_water(world: &PaletteWorld<'_>, eye: [f32; 3]) -> bool {
    use sim::CollisionWorld;
    let block = eye.map(|axis| axis.floor() as i32);
    world.block_physics(block).map_or(true, |sample| {
        sample.layers.iter().any(|layer| {
            layer.flags.contains(sim::BlockPhysicsFlags::WATER)
                && f64::from(eye[1]) < f64::from(block[1]) + layer.fluid_height_blocks
        })
    })
}

#[cfg(test)]
pub(crate) mod tests;
