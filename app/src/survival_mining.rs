//! Survival hold-to-mine: one destroy state-machine step per completed physics tick.
//!
//! Completion is only predicted from the provisional destroy table; inbound block
//! updates remain the sole block-change authority. Unknown blocks keep cracking
//! without a prediction so the server decides when they break.

use bevy::{
    ecs::system::SystemParam,
    prelude::{Query, Res, ResMut, Resource, Window, With},
    window::PrimaryWindow,
};
use protocol::{
    BlockAction, BlockActionKind, BlockActions, BlockBreakingAuthority, BlockItemInteraction,
    BlockUseRequest, PlayerAuthInputInteractions, PlayerGameMode,
};
use semantic_input::Action;
use sim::{BlockDestroyInfo, DestroyConditions, HeldTool, PaletteWorld};

use crate::{
    interaction_authority::observe_block,
    local_player::InteractionOriginSnapshot,
    menu::MenuRuntime,
    mining::{FrozenMiningSelection, protocol_input_mode, survival_reach, verified_selection},
    movement::{LocalMovementEffectTimeline, MovementTicker, PhysicsCollisionRegistries},
    runtime::world::ClientWorld,
    semantic_controls::SemanticInputSnapshot,
    ui_runtime::UiRuntime,
};

/// Ticks between a completed destroy and the next start. Needs independent measurement.
pub(crate) const DESTROY_DELAY_TICKS: u8 = 5;
/// How long a predicted break suppresses restarting on the unchanged block.
const PREDICTED_BREAK_HOLD_TICKS: u8 = 20;

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
}

impl DestroyTarget {
    fn rate(&self, on_ground: bool) -> Option<f32> {
        let conditions = DestroyConditions {
            on_ground,
            ..self.conditions
        };
        sim::destroy_progress_per_tick(self.block.as_ref()?, &conditions)
    }
}

/// Destroy actions for one tick, plus the client-authoritative completion if any.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct SurvivalTickPayload {
    pub(crate) actions: BlockActions,
    pub(crate) destroy: Option<DestroyTarget>,
}

impl SurvivalTickPayload {
    pub(crate) fn is_empty(&self) -> bool {
        self.actions.is_empty() && self.destroy.is_none()
    }

    /// Faces, percents, hit offsets and slots are bounded at their sources, so
    /// the carrier's own validation cannot reject the result.
    pub(crate) fn into_interactions(
        self,
        player_position: [f32; 3],
    ) -> PlayerAuthInputInteractions {
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
        PlayerAuthInputInteractions {
            block_actions: self.actions,
            block_interaction,
        }
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
    progress: f32,
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
            DestroyInput::Held(Some(target)) => target,
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
                if target.block.is_some_and(|block| block.hardness == 0.0) {
                    self.complete(&mut payload, target, authority, false);
                    self.delay = DESTROY_DELAY_TICKS;
                } else if authority == BlockBreakingAuthority::Client {
                    payload.push(BlockActionKind::CrackBlock, target.position, target.face);
                }
            }
            Some(destroying) if destroying.position == target.position => {
                let rate = target.rate(on_ground).unwrap_or(0.0);
                let progress = destroying.progress + rate;
                if progress >= 1.0 {
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
    fn complete(
        &mut self,
        payload: &mut SurvivalTickPayload,
        target: &DestroyTarget,
        authority: BlockBreakingAuthority,
        continued: bool,
    ) {
        match authority {
            BlockBreakingAuthority::Server => {
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

/// Survival destroy sequencing bound to the current position authority.
#[derive(Resource, Debug, Default)]
pub(crate) struct SurvivalMiningRuntime {
    machine: DestroyMachine,
    last_stepped_tick: Option<u64>,
    latched_press: bool,
    position_authority: Option<(u64, u64)>,
}

impl SurvivalMiningRuntime {
    /// Steps every unsent tick once, attaching nonempty payloads to their samples.
    pub(crate) fn step_ticks(
        &mut self,
        ticker: &mut MovementTicker,
        input: DestroyInput<'_>,
        authority: BlockBreakingAuthority,
    ) {
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
            return;
        };
        self.last_stepped_tick = Some(newest);
        if !ticker.accepts_creative_mining() {
            // Withheld ticks never reach the server, so neither may their actions.
            self.machine.interrupt();
            return;
        }
        for (tick, on_ground) in ticks {
            let payload = self.machine.step(input, on_ground, authority);
            self.latched_press = false;
            if !payload.is_empty() && !ticker.attach_survival_mining(tick, payload) {
                // A tick that cannot carry its actions desynchronizes the server's view.
                self.machine.interrupt();
            }
        }
    }
}

#[derive(SystemParam)]
pub(crate) struct SurvivalMiningContext<'w, 's> {
    input: Res<'w, SemanticInputSnapshot>,
    origin: Res<'w, InteractionOriginSnapshot>,
    ui: Res<'w, UiRuntime>,
    menu: Res<'w, MenuRuntime>,
    windows: Query<'w, 's, &'static Window, With<PrimaryWindow>>,
    client_world: Res<'w, ClientWorld>,
    collisions: Res<'w, PhysicsCollisionRegistries>,
    effects: Res<'w, LocalMovementEffectTimeline>,
}

/// Runs after committed world publication and before the movement flush.
pub(crate) fn produce_survival_mining(
    context: SurvivalMiningContext,
    mut runtime: ResMut<SurvivalMiningRuntime>,
    mut movement: ResMut<MovementTicker>,
) {
    let authority = context.ui.block_breaking_authority();
    let focused =
        !context.menu.is_visible() && context.windows.single().is_ok_and(|window| window.focused);
    let (Some(authority), Some(input), true) = (authority, context.input.snapshot(), focused)
    else {
        runtime.latched_press = false;
        runtime.step_ticks(
            &mut movement,
            DestroyInput::Released,
            authority.unwrap_or(BlockBreakingAuthority::Server),
        );
        return;
    };
    let attack = context.input.phase(Action::Attack);
    runtime.latched_press |= attack.pressed;
    if !attack.held && !runtime.latched_press {
        runtime.step_ticks(&mut movement, DestroyInput::Released, authority);
        return;
    }
    let target = observe_destroy_target(
        &context,
        input.input_mode,
        (input.authority_generation, input.frame_sequence),
        movement.interaction_authority_identity().1,
    );
    runtime.step_ticks(
        &mut movement,
        DestroyInput::Held(target.as_ref()),
        authority,
    );
}

fn observe_destroy_target(
    context: &SurvivalMiningContext,
    input_mode: semantic_input::InputMode,
    input_authority: (std::num::NonZeroU64, u64),
    position_authority_generation: u64,
) -> Option<DestroyTarget> {
    let ui = &context.ui;
    if ui.ui_focused() || ui.player_game_mode()? != PlayerGameMode::Survival {
        return None;
    }
    let selection = verified_selection(ui)?;
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
    // Vanilla limits the pick by the eye-to-block-centre distance, not the ray length.
    let centre_distance_squared = observed
        .target
        .position
        .into_iter()
        .zip(observed.ray.origin)
        .map(|(block, eye)| (f64::from(block) + 0.5 - f64::from(eye)).powi(2))
        .sum::<f64>();
    if centre_distance_squared > observed.reach * observed.reach {
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
    Some(DestroyTarget {
        position: observed.target.position,
        face: observed.target.face,
        runtime_id: observed.target.runtime_id,
        relative_hit: observed.target.relative_hit,
        block,
        conditions: DestroyConditions {
            tool,
            // Enchantments are not decoded; omitting them only delays prediction.
            efficiency_level: 0,
            haste_amplifier: effects.haste,
            conduit_power_amplifier: effects.conduit_power,
            mining_fatigue_amplifier: effects.mining_fatigue,
            on_ground: true,
            riding: ui.gameplay_hud().mount_unique_id().is_some(),
            eyes_in_water: eyes_in_water(&world, observed.ray.origin),
            aqua_affinity: false,
        },
        selection: observed.selection,
    })
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
mod tests;
