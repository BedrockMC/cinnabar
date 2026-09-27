//! Block use as standalone click-block transactions on the press and while held.
//!
//! Placement is predicted only for a block item into loaded air clear of the
//! player; every outcome stays server-owned. Air use, item-use-on start/stop
//! actions and bridging repeat timing are not implemented.

use bevy::{
    ecs::system::SystemParam,
    prelude::{Query, Real, Res, ResMut, Resource, Time, Window, With},
    window::PrimaryWindow,
};
use protocol::{
    BlockUseRequest, ItemUseTrigger, PlayerGameMode, PlayerInputMode, SwingSource,
    VerifiedNetworkItemStack,
};
use semantic_input::Action;
use sim::PaletteWorld;

use crate::{
    interaction_authority::{FrozenBlockObservation, observe_block, within_pick_range},
    local_player::InteractionOriginSnapshot,
    melee::{MeleeRuntime, SwingTracker, swing_duration},
    menu::MenuRuntime,
    mining::{
        FrozenMiningSelection, creative_reach, protocol_input_mode, survival_reach,
        verified_selection,
    },
    movement::{LocalMovementEffectTimeline, MovementTicker, PhysicsCollisionRegistries},
    runtime::{network::NetworkHandle, world::ClientWorld},
    semantic_controls::SemanticInputSnapshot,
    ui_runtime::UiRuntime,
};

// Held-use repeat timing is wall-clock. All values need independent measurement.
const SNEAKING_REPEAT_MILLIS: u64 = 300;
const STILL_REPEAT_MILLIS: u64 = 200;
const MOVING_REPEAT_MAX_MILLIS: u64 = 180;
/// Moving repeats wait this many milliseconds per block/second of speed.
const MOVING_REPEAT_BLOCK_MILLIS: f32 = 900.0;
const SURVIVAL_REPEAT_FLOOR_MILLIS: u64 = 100;
/// Speeds below this count as standing still, in blocks per second.
const STILL_SPEED: f32 = 0.01;

/// Milliseconds until the next held-use repeat.
pub(crate) fn repeat_interval_millis(sneaking: bool, speed: f32, survival: bool) -> u64 {
    let interval = if sneaking {
        SNEAKING_REPEAT_MILLIS
    } else if !speed.is_finite() || speed < STILL_SPEED {
        STILL_REPEAT_MILLIS
    } else {
        ((MOVING_REPEAT_BLOCK_MILLIS / speed) as u64).min(MOVING_REPEAT_MAX_MILLIS)
    };
    if survival {
        interval.max(SURVIVAL_REPEAT_FLOOR_MILLIS)
    } else {
        interval
    }
}

/// The cell a block placed against `face` of `clicked` would occupy.
pub(crate) const fn placement_cell(clicked: [i32; 3], face: u8) -> [i32; 3] {
    let [x, y, z] = clicked;
    match face {
        0 => [x, y - 1, z],
        1 => [x, y + 1, z],
        2 => [x, y, z - 1],
        3 => [x, y, z + 1],
        4 => [x - 1, y, z],
        _ => [x + 1, y, z],
    }
}

/// Whether local placement succeeds: a block item into air outside the player box.
pub(crate) fn predict_placement(
    item: &VerifiedNetworkItemStack,
    cell: [i32; 3],
    cell_is_air: bool,
    player_feet: [f32; 3],
) -> bool {
    let half_width = sim::PLAYER_WIDTH * 0.5;
    let low = [
        f64::from(player_feet[0]) - half_width,
        f64::from(player_feet[1]),
        f64::from(player_feet[2]) - half_width,
    ];
    let high = [
        f64::from(player_feet[0]) + half_width,
        f64::from(player_feet[1]) + sim::PLAYER_HEIGHT,
        f64::from(player_feet[2]) + half_width,
    ];
    let overlaps_player = (0..3).all(|axis| {
        let cell_low = f64::from(cell[axis]);
        cell_low < high[axis] && low[axis] < cell_low + 1.0
    });
    item.block_runtime_id() != 0 && item.count() > 0 && cell_is_air && !overlaps_player
}

/// Press latch and held-repeat schedule.
#[derive(Resource, Debug, Default)]
pub(crate) struct BlockUseRuntime {
    latched_press: bool,
    next_repeat_millis: Option<u64>,
}

impl BlockUseRuntime {
    fn clear(&mut self) {
        self.latched_press = false;
        self.next_repeat_millis = None;
    }

    /// The trigger due this frame, if any.
    pub(crate) fn due(&self, held: bool, now_millis: u64) -> Option<ItemUseTrigger> {
        if self.latched_press {
            Some(ItemUseTrigger::PlayerInput)
        } else if held
            && self
                .next_repeat_millis
                .is_some_and(|next| now_millis >= next)
        {
            Some(ItemUseTrigger::SimulationTick)
        } else {
            None
        }
    }
}

#[derive(SystemParam)]
pub(crate) struct BlockUseContext<'w, 's> {
    input: Res<'w, SemanticInputSnapshot>,
    origin: Res<'w, InteractionOriginSnapshot>,
    ui: Res<'w, UiRuntime>,
    menu: Res<'w, MenuRuntime>,
    windows: Query<'w, 's, &'static Window, With<PrimaryWindow>>,
    client_world: Res<'w, ClientWorld>,
    collisions: Res<'w, PhysicsCollisionRegistries>,
    effects: Res<'w, LocalMovementEffectTimeline>,
    melee: Res<'w, MeleeRuntime>,
    network: Res<'w, NetworkHandle>,
    time: Res<'w, Time<Real>>,
}

pub(crate) fn produce_block_use(
    context: BlockUseContext,
    mut runtime: ResMut<BlockUseRuntime>,
    mut swings: ResMut<SwingTracker>,
    movement: Res<MovementTicker>,
) {
    let focused =
        !context.menu.is_visible() && context.windows.single().is_ok_and(|window| window.focused);
    let game_mode = context.ui.player_game_mode();
    let Some(input) = context.input.snapshot().filter(|input| {
        focused
            && !context.ui.ui_focused()
            && matches!(
                game_mode,
                Some(PlayerGameMode::Survival | PlayerGameMode::Creative)
            )
            && input.input_mode != semantic_input::InputMode::Touch
            && movement.accepts_creative_mining()
    }) else {
        runtime.clear();
        return;
    };
    let use_phase = context.input.phase(Action::Use);
    if context.input.phase(Action::Attack).held || !(use_phase.held || use_phase.pressed) {
        runtime.clear();
        return;
    }
    runtime.latched_press |= use_phase.pressed;
    let now_millis = u64::try_from(context.time.elapsed().as_millis()).unwrap_or(u64::MAX);
    let Some(trigger) = runtime.due(use_phase.held, now_millis) else {
        return;
    };
    let Some(sample) = movement.newest_unsent_sample() else {
        return;
    };
    runtime.latched_press = false;
    let survival = game_mode == Some(PlayerGameMode::Survival);
    let speed = sample.delta[0].hypot(sample.delta[2]) * sim::TICKS_PER_SECOND as f32;
    runtime.next_repeat_millis =
        Some(now_millis.saturating_add(repeat_interval_millis(sample.sneaking, speed, survival)));
    if context.melee.blocks_use_at(sample.tick) {
        return;
    }
    let input_mode = protocol_input_mode(input.input_mode);
    let Some(observed) = observe_use_target(
        &context,
        input_mode,
        survival,
        (input.authority_generation, input.frame_sequence),
        movement.interaction_authority_identity().1,
    ) else {
        return;
    };
    let item = &observed.selection.item;
    // Only block items keep placing while held.
    if trigger == ItemUseTrigger::SimulationTick && item.block_runtime_id() == 0 {
        return;
    }
    let Some(stream) = context.client_world.stream.as_ref() else {
        return;
    };
    let cell = placement_cell(observed.target.position, observed.target.face);
    let world = PaletteWorld::new(
        stream.collision_store(),
        context.collisions.registry(stream.network_id_mode()),
        stream.current_dimension(),
    );
    let feet = [
        sample.position[0],
        sample.position[1] - protocol::PLAYER_NETWORK_OFFSET,
        sample.position[2],
    ];
    let predicted = predict_placement(item, cell, world.is_air(cell) == Ok(true), feet);
    // A successful local use swings before its transaction.
    if predicted
        && swings.try_swing(
            sample.tick,
            swing_duration(context.effects.mining_effects()),
        )
    {
        let _ = context
            .network
            .send_interaction_packet(protocol::swing_arm_packet(
                stream.local_player_runtime_id(),
                SwingSource::Build,
            ));
    }
    let request = BlockUseRequest {
        block_position: observed.target.position,
        face: observed.target.face,
        selected_slot: observed.selection.slot,
        selected_item: observed.selection.item,
        player_position: sample.position,
        relative_hit: observed.target.relative_hit,
        block_runtime_id: u64::from(observed.target.runtime_id),
    };
    if let Ok(packet) = protocol::click_block_transaction_packet(request, trigger, predicted) {
        let _ = context.network.send_interaction_packet(packet);
    }
}

fn observe_use_target(
    context: &BlockUseContext,
    input_mode: PlayerInputMode,
    survival: bool,
    input_authority: (std::num::NonZeroU64, u64),
    position_authority_generation: u64,
) -> Option<FrozenBlockObservation> {
    let reach = if survival {
        survival_reach(input_mode)
    } else {
        creative_reach(input_mode)
    };
    let observed = observe_block(
        &context.origin,
        &context.ui,
        &context.client_world,
        &context.collisions,
        verified_use_selection(&context.ui)?,
        (
            input_mode,
            reach,
            input_authority,
            position_authority_generation,
        ),
    )?;
    within_pick_range(&observed).then_some(observed)
}

/// The selected stack, only while no inventory request or hotbar change is in flight.
fn verified_use_selection(ui: &UiRuntime) -> Option<FrozenMiningSelection> {
    let ledger = ui.inventory_ledger();
    if ledger.pending_request_id().is_some()
        || ledger.resync_required()
        || ui.pending_hotbar_selection().is_some()
    {
        return None;
    }
    verified_selection(ui)
}

#[cfg(test)]
mod tests;
