//! Creative block-mining production and immutable tick attachment.

use std::num::NonZeroU64;

use bevy::{
    ecs::system::SystemParam,
    prelude::{Query, Res, ResMut, Resource, Window, With},
    window::PrimaryWindow,
};
use protocol::{
    BlockAction, BlockActionKind, BlockActions, BlockItemInteraction, BlockUseRequest,
    PlayerAuthInputInteractions, PlayerInputMode, VerifiedNetworkItemStack,
};
use semantic_input::{Action, InputMode};
use sim::WorldCollisionIdentity;

use crate::{
    local_player::InteractionOriginSnapshot,
    menu::MenuRuntime,
    movement::{MovementTicker, PhysicsCollisionRegistries},
    runtime::world::ClientWorld,
    semantic_controls::SemanticInputSnapshot,
    ui_runtime::{UiRuntime, inventory_ledger::PlayerInventorySlot},
};

/// Creative pick ranges observed for the three input modes the app exposes.
/// They are deliberately isolated here; survival timing and reach remain out
/// of this first interaction slice until their complete authority exists.
const CREATIVE_MOUSE_REACH_BLOCKS: f64 = 5.7;
const CREATIVE_GAMEPAD_REACH_BLOCKS: f64 = 5.6;
const CREATIVE_TOUCH_REACH_BLOCKS: f64 = 12.0;
/// Survival keeps the mouse and gamepad ranges; touch is shorter. Needs independent measurement.
const SURVIVAL_TOUCH_REACH_BLOCKS: f64 = 6.7;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CreativeMiningAbility {
    InstantBreak,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct FrozenMiningFrame {
    pub(crate) session_generation: u64,
    pub(crate) position_authority_generation: u64,
    pub(crate) input_authority_generation: NonZeroU64,
    pub(crate) input_frame_sequence: u64,
    pub(crate) fifo_sequence: u64,
    pub(crate) physics_tick: u64,
    pub(crate) pose_generation: u64,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct FrozenMiningRay {
    pub(crate) origin: [f32; 3],
    pub(crate) direction: [f32; 3],
    pub(crate) movement_world_identity: WorldCollisionIdentity,
    pub(crate) world_identity: WorldCollisionIdentity,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FrozenMiningSelection {
    pub(crate) slot: u8,
    pub(crate) item: VerifiedNetworkItemStack,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct FrozenMiningTarget {
    pub(crate) position: [i32; 3],
    pub(crate) face: u8,
    pub(crate) relative_hit: [f32; 3],
    pub(crate) runtime_id: u32,
    pub(crate) identity: WorldCollisionIdentity,
}

/// Complete immutable authority behind one creative break candidate.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct FrozenCreativeMining {
    pub(crate) frame: FrozenMiningFrame,
    pub(crate) ray: FrozenMiningRay,
    pub(crate) reach: f64,
    pub(crate) input_mode: PlayerInputMode,
    pub(crate) ability: CreativeMiningAbility,
    pub(crate) selection: FrozenMiningSelection,
    pub(crate) target: FrozenMiningTarget,
}

impl FrozenCreativeMining {
    pub(crate) fn still_authorized_by(&self, current: &Self) -> bool {
        self.ability == current.ability
            && crate::interaction_authority::still_authorized_by(
                (
                    &self.frame,
                    &self.ray,
                    self.reach,
                    self.input_mode,
                    &self.selection,
                    &self.target,
                ),
                (
                    &current.frame,
                    &current.ray,
                    current.reach,
                    current.input_mode,
                    &current.selection,
                    &current.target,
                ),
            )
    }

    pub(crate) fn into_tick_payload(self, player_position: [f32; 3]) -> QueuedMiningInteraction {
        let target = &self.target;
        let mut block_actions = BlockActions::new();
        for kind in [
            BlockActionKind::StartDestroy,
            BlockActionKind::PredictDestroy,
        ] {
            block_actions
                .push(BlockAction {
                    kind,
                    position: target.position,
                    face: target.face,
                })
                .expect("one creative break uses two of eight bounded block actions");
        }
        let interactions = PlayerAuthInputInteractions {
            block_actions,
            block_interaction: Some(BlockItemInteraction::Destroy(BlockUseRequest {
                block_position: target.position,
                face: target.face,
                selected_slot: self.selection.slot,
                selected_item: self.selection.item.clone(),
                player_position,
                relative_hit: target.relative_hit,
                block_runtime_id: u64::from(target.runtime_id),
            })),
        };
        QueuedMiningInteraction {
            authority: Some(self),
            interactions,
            mining_request: None,
        }
    }
}

/// Concrete payload retained on a completed movement tick through retries.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct QueuedMiningInteraction {
    /// `None` for a survival tick already committed by its destroy state machine.
    authority: Option<FrozenCreativeMining>,
    pub(crate) interactions: PlayerAuthInputInteractions,
    pub(crate) mining_request: Option<protocol::MineBlockRequest>,
}

impl QueuedMiningInteraction {
    pub(crate) const fn survival(
        interactions: PlayerAuthInputInteractions,
        mining_request: Option<protocol::MineBlockRequest>,
    ) -> Self {
        Self {
            authority: None,
            interactions,
            mining_request,
        }
    }

    pub(crate) const fn is_creative(&self) -> bool {
        self.authority.is_some()
    }

    pub(crate) fn still_authorized_by(&self, current: &FrozenCreativeMining) -> bool {
        self.authority
            .as_ref()
            .is_none_or(|authority| authority.still_authorized_by(current))
    }
}

#[derive(Debug, Clone, PartialEq)]
struct PendingAttackPress {
    authority: FrozenCreativeMining,
}

/// One bounded attack edge waiting for an exact completed physics tick.
#[derive(Resource, Debug, Default)]
pub(crate) struct MiningRuntime {
    pending_press: Option<PendingAttackPress>,
    position_authority: Option<(u64, u64)>,
}

impl MiningRuntime {
    fn synchronize_position_authority(&mut self, ticker: &mut MovementTicker) -> bool {
        let position_authority = ticker.mining_authority_identity();
        let changed = self
            .position_authority
            .is_some_and(|previous| previous != position_authority);
        self.position_authority = Some(position_authority);
        if changed {
            self.pending_press = None;
            ticker.retain_creative_mining(None);
        }
        changed
    }

    fn update_press(
        &mut self,
        pressed: bool,
        input_authority_generation: NonZeroU64,
        current: Option<FrozenCreativeMining>,
        ticker: &mut MovementTicker,
    ) -> Option<u64> {
        if self.synchronize_position_authority(ticker) {
            return None;
        }
        if self.pending_press.as_ref().is_some_and(|pending| {
            pending.authority.frame.input_authority_generation != input_authority_generation
        }) {
            self.pending_press = None;
        }
        ticker.retain_creative_mining(current.as_ref());
        if !ticker.accepts_creative_mining() {
            self.pending_press = None;
            return None;
        }
        if pressed {
            self.pending_press = current.as_ref().map(|authority| PendingAttackPress {
                authority: authority.clone(),
            });
        }

        let pending = self.pending_press.as_ref()?;
        if pending.authority.frame.input_authority_generation != input_authority_generation {
            self.pending_press = None;
            return None;
        }
        let Some(current) = current else {
            self.pending_press = None;
            return None;
        };
        if !pending.authority.still_authorized_by(&current) {
            self.pending_press = None;
            return None;
        }
        let mut attachment = pending.authority.clone();
        attachment.frame.physics_tick = current.frame.physics_tick;
        let attached = ticker.attach_creative_mining(attachment);
        if attached.is_some() {
            self.pending_press = None;
        }
        attached
    }

    #[cfg(test)]
    const fn has_pending_press(&self) -> bool {
        self.pending_press.is_some()
    }
}

#[derive(SystemParam)]
pub(crate) struct CreativeMiningContext<'w, 's> {
    input: Res<'w, SemanticInputSnapshot>,
    origin: Res<'w, InteractionOriginSnapshot>,
    ui: Res<'w, UiRuntime>,
    menu: Res<'w, MenuRuntime>,
    windows: Query<'w, 's, &'static Window, With<PrimaryWindow>>,
    client_world: Res<'w, ClientWorld>,
    collisions: Res<'w, PhysicsCollisionRegistries>,
    melee: Res<'w, crate::melee::MeleeRuntime>,
}

/// Produces at most one instant creative break per fresh attack press.
///
/// This runs after committed world publication and immediately before the
/// movement flush. It never mutates the local world; inbound server block
/// updates remain the sole block-change authority.
pub(crate) fn produce_creative_mining(
    context: CreativeMiningContext,
    mut runtime: ResMut<MiningRuntime>,
    mut movement: ResMut<MovementTicker>,
) {
    let position_authority_changed = runtime.synchronize_position_authority(&mut movement);
    if !creative_mining_input_authorized(
        context.menu.is_visible(),
        context.windows.single().is_ok_and(|window| window.focused),
    ) {
        runtime.pending_press = None;
        movement.retain_creative_mining(None);
        return;
    }
    let Some(input_snapshot) = context.input.snapshot() else {
        runtime.pending_press = None;
        movement.retain_creative_mining(None);
        return;
    };
    let raw_attack_pressed = context.input.phase(Action::Attack).pressed;
    let use_pressed = context.input.phase(Action::Use).pressed;
    if raw_attack_pressed && use_pressed {
        runtime.pending_press = None;
        movement.retain_creative_mining(None);
        return;
    }
    // A press on an actor in front of the block is an attack, not a break.
    let attack_pressed = !position_authority_changed
        && !context.melee.actor_in_front()
        && mining_edge_authorized(raw_attack_pressed, use_pressed);
    if !attack_pressed && runtime.pending_press.is_none() && !movement.has_queued_creative_mining()
    {
        return;
    }
    let position_authority_generation = movement.mining_authority_identity().1;
    let current = creative_observation(
        &context.origin,
        &context.ui,
        &context.client_world,
        &context.collisions,
        input_snapshot.input_mode,
        (
            input_snapshot.authority_generation,
            input_snapshot.frame_sequence,
        ),
        position_authority_generation,
    );
    let _ = runtime.update_press(
        attack_pressed,
        input_snapshot.authority_generation,
        current,
        &mut movement,
    );
}

pub(crate) fn creative_observation(
    origin: &InteractionOriginSnapshot,
    ui: &UiRuntime,
    client_world: &ClientWorld,
    collisions: &PhysicsCollisionRegistries,
    input_mode: InputMode,
    input_authority: (NonZeroU64, u64),
    position_authority_generation: u64,
) -> Option<FrozenCreativeMining> {
    let ability = creative_mining_ui_ability(ui.ui_focused(), ui.player_game_mode()?)?;
    let input_mode = protocol_input_mode(input_mode);
    let observed = crate::interaction_authority::observe_block(
        origin,
        ui,
        client_world,
        collisions,
        verified_selection(ui)?,
        (
            input_mode,
            creative_reach(input_mode),
            input_authority,
            position_authority_generation,
        ),
    )?;
    Some(FrozenCreativeMining {
        frame: observed.frame,
        ray: observed.ray,
        reach: observed.reach,
        input_mode: observed.input_mode,
        ability,
        selection: observed.selection,
        target: observed.target,
    })
}

pub(crate) fn verified_selection(ui: &UiRuntime) -> Option<FrozenMiningSelection> {
    let selected = ui.selected_stack_snapshot()?;
    let stack = match selected.state {
        // Before the inventory arrives the slot is unknown; vanilla assumes an
        // empty hand until restated, so mining works by hand from the first tick.
        PlayerInventorySlot::Unknown | PlayerInventorySlot::Empty => {
            protocol::NetworkItemStack::empty()
        }
        PlayerInventorySlot::Present(stack) => stack.clone(),
    };
    let item = VerifiedNetworkItemStack::try_new(stack.clone(), stack.nbt_digest).ok()?;
    Some(FrozenMiningSelection {
        slot: selected.slot,
        item,
    })
}

const fn creative_mining_ability(
    game_mode: protocol::PlayerGameMode,
) -> Option<CreativeMiningAbility> {
    match game_mode {
        protocol::PlayerGameMode::Creative => Some(CreativeMiningAbility::InstantBreak),
        protocol::PlayerGameMode::Survival
        | protocol::PlayerGameMode::Adventure
        | protocol::PlayerGameMode::Spectator
        | protocol::PlayerGameMode::Unknown => None,
    }
}

const fn creative_mining_ui_ability(
    ui_focused: bool,
    game_mode: protocol::PlayerGameMode,
) -> Option<CreativeMiningAbility> {
    if ui_focused {
        None
    } else {
        creative_mining_ability(game_mode)
    }
}

const fn mining_edge_authorized(attack_pressed: bool, use_pressed: bool) -> bool {
    attack_pressed && !use_pressed
}

const fn creative_mining_input_authorized(menu_visible: bool, window_focused: bool) -> bool {
    !menu_visible && window_focused
}

pub(crate) const fn protocol_input_mode(input_mode: InputMode) -> PlayerInputMode {
    match input_mode {
        InputMode::KeyboardMouse => PlayerInputMode::Mouse,
        InputMode::GamePad => PlayerInputMode::GamePad,
        InputMode::Touch => PlayerInputMode::Touch,
    }
}

pub(crate) const fn survival_reach(input_mode: PlayerInputMode) -> f64 {
    match input_mode {
        PlayerInputMode::Touch => SURVIVAL_TOUCH_REACH_BLOCKS,
        PlayerInputMode::Mouse | PlayerInputMode::GamePad => creative_reach(input_mode),
    }
}

pub(crate) const fn creative_reach(input_mode: PlayerInputMode) -> f64 {
    match input_mode {
        PlayerInputMode::Mouse => CREATIVE_MOUSE_REACH_BLOCKS,
        PlayerInputMode::GamePad => CREATIVE_GAMEPAD_REACH_BLOCKS,
        PlayerInputMode::Touch => CREATIVE_TOUCH_REACH_BLOCKS,
    }
}

#[cfg(test)]
mod tests;
