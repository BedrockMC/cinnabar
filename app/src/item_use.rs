//! Air item use: the click-air transaction every held item sends, and holding, releasing and
//! throwing.
//!
//! Follows `ClientInputCallbacks::handleBuildAction`, `GameMode::baseUseItem`,
//! `GameMode::releaseUsingItem` and `Player::completeUsingItem`; projectiles, food effects and
//! ammunition stay server-owned.

use bevy::{
    ecs::system::SystemParam,
    prelude::{Query, Real, Res, ResMut, Resource, Time, Window, With},
    window::PrimaryWindow,
};
use client_world::{LocalItemUse, WorldStream};
use protocol::{HeldItemRequest, PlayerGameMode, PredictedSlotChange, VerifiedNetworkItemStack};
use semantic_input::Action;

use crate::{
    block_use::{BlockUseRuntime, verified_use_selection},
    melee::{MeleeRuntime, SwingTracker, swing_duration},
    menu::MenuRuntime,
    mining::FrozenMiningSelection,
    movement::{LocalMovementEffectTimeline, MovementTicker},
    runtime::{
        network::{BatchSendError, NetworkHandle},
        world::ClientWorld,
    },
    semantic_controls::SemanticInputSnapshot,
    ui_runtime::UiRuntime,
};

mod admission;
mod classify;
use admission::step_and_send;
pub(crate) use classify::{AirUse, Cooldown, Needs, classify};

const QUICK_CHARGE_ENCHANTMENT_ID: i16 = 35;
/// `handleBuildAction` re-arms the next build action this long after an air use.
const USE_REARM_MILLIS: u64 = 200;

#[derive(Debug, Clone, PartialEq)]
struct ActiveUse {
    selection: FrozenMiningSelection,
    started_tick: u64,
    max_ticks: u32,
    slowdown: f64,
}

/// A throw's locally consumed stack, shown until the server restates the slot.
#[derive(Debug, Clone, PartialEq)]
struct PredictedStack {
    slot: u8,
    /// The server's stack when the throw was predicted.
    server: VerifiedNetworkItemStack,
    stack: VerifiedNetworkItemStack,
}

/// One unsent tick's view of the use input and the selected stack.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct UseFrame {
    pub(crate) tick: u64,
    pub(crate) now_millis: u64,
    pub(crate) position: [f32; 3],
    pub(crate) held: bool,
    pub(crate) selection: Option<FrozenMiningSelection>,
    pub(crate) air_use: Option<AirUse>,
    /// The use's `Needs` are met (always true in creative).
    pub(crate) ready: bool,
    pub(crate) creative: bool,
    /// A block interaction or recent attack consumed this press.
    pub(crate) press_consumed: bool,
}

/// Transactions in send order, plus what the local player did on this tick.
#[derive(Debug, Default)]
pub(crate) struct UseOutcome {
    pub(crate) packets: Vec<protocol::Packet>,
    pub(crate) started: bool,
    /// A throw swung the arm; its swing packet precedes `packets`.
    pub(crate) swung: bool,
    used: bool,
    released: bool,
}

/// The press latch, the accepted use, cooldowns and the throw prediction.
#[derive(Resource, Debug, Default, Clone)]
pub(crate) struct ItemUseRuntime {
    latched_press: bool,
    active: Option<ActiveUse>,
    session: Option<u64>,
    rearm_millis: Option<u64>,
    /// Cooldown category and the tick it ends.
    cooldowns: Vec<(&'static str, u64)>,
    predicted: Option<PredictedStack>,
    /// The use button has stayed down since a press no block interaction consumed.
    repeat_armed: bool,
    /// A rejected click retries only while its verified selection remains current.
    deferred_selection: Option<FrozenMiningSelection>,
    /// A rejected release still precedes the next use, even if Use is pressed again.
    release_pending: bool,
    /// `TypedClientNetId<ItemStackLegacyRequestIdTag>`'s process-wide counter.
    last_legacy_request_id: i32,
}

impl ItemUseRuntime {
    /// Whether a use started locally and has not ended.
    pub(crate) const fn is_using(&self) -> bool {
        self.active.is_some()
    }

    /// Movement-input factor while a use runs; `None` when idle.
    pub(crate) fn movement_modifier(&self) -> Option<f64> {
        self.active.as_ref().map(|active| active.slowdown)
    }

    /// A new session drops the press, the use, cooldowns and the prediction without packets.
    pub(crate) fn synchronize(&mut self, session: u64) {
        if self.session.is_some_and(|previous| previous != session) {
            self.cancel();
        }
        self.session = Some(session);
    }

    pub(crate) fn observe_press(&mut self, pressed: bool) {
        if pressed {
            self.deferred_selection = None;
            self.latched_press = true;
        }
    }

    /// Screens, focus loss and spectator mode cancel queued clicks, but accepted uses release.
    pub(crate) fn cancel_pending_input(&mut self) {
        self.latched_press = false;
        self.repeat_armed = false;
        self.deferred_selection = None;
    }

    fn cancel(&mut self) {
        self.cancel_pending_input();
        self.active = None;
        self.rearm_millis = None;
        self.cooldowns.clear();
        self.predicted = None;
        self.release_pending = false;
    }

    /// Whether this frame has anything to resolve against an unsent tick.
    pub(crate) const fn has_work(&self, held: bool) -> bool {
        self.latched_press || self.active.is_some() || held
    }

    /// Ends a use on release, depletion or reselection, then resolves a press or held repeat.
    pub(crate) fn step(&mut self, frame: &UseFrame) -> UseOutcome {
        let mut outcome = UseOutcome::default();
        let pressed = std::mem::take(&mut self.latched_press);
        if pressed {
            self.repeat_armed = !frame.press_consumed;
        }
        self.cooldowns.retain(|(_, until)| frame.tick < *until);
        let release_pending = std::mem::take(&mut self.release_pending);
        self.end_use(frame, &mut outcome, release_pending);
        if self.active.is_none() && (pressed || frame.held) {
            self.try_use(frame, pressed, &mut outcome);
        }
        if !frame.held {
            self.repeat_armed = false;
        }
        outcome
    }

    fn end_use(&mut self, frame: &UseFrame, outcome: &mut UseOutcome, release_pending: bool) {
        let Some(active) = &self.active else {
            return;
        };
        let selection = match &frame.selection {
            Some(current)
                if current.slot != active.selection.slot
                    || current.item.network_id() != active.selection.item.network_id() =>
            {
                // Switching away stops the use without a release, as `Player::stopUsingItem`.
                self.active = None;
                return;
            }
            Some(current) => current.clone(),
            // An in-flight inventory request hides the stack; keep the one the use began with.
            None => active.selection.clone(),
        };
        if frame.held && !release_pending {
            // A depleted use completes locally; the client sends nothing for it.
            if frame.tick.saturating_sub(active.started_tick) >= u64::from(active.max_ticks) {
                self.active = None;
            }
            return;
        }
        self.active = None;
        if let Ok(packet) = protocol::release_item_packet(held_request(&selection, frame)) {
            outcome.packets.push(packet);
            outcome.released = true;
        }
    }

    /// Why a latched press sends nothing on this frame, for the click-drop trace; `None` when it
    /// is used or no press waits.
    pub(crate) fn press_drop_reason(&self, frame: &UseFrame) -> Option<&'static str> {
        if !self.latched_press || self.active.is_some() {
            return None;
        }
        if frame.press_consumed {
            Some("consumed_by_block_or_attack")
        } else if self
            .rearm_millis
            .is_some_and(|rearm| frame.now_millis <= rearm)
        {
            Some("rearm_pending")
        } else if frame.selection.is_none() {
            Some("selection_unverified")
        } else if frame
            .selection
            .as_ref()
            .is_some_and(|selection| selection.item.is_empty())
        {
            Some("no_air_use_for_item")
        } else {
            None
        }
    }

    fn try_use(&mut self, frame: &UseFrame, pressed: bool, outcome: &mut UseOutcome) {
        if frame.press_consumed
            || self
                .rearm_millis
                .is_some_and(|rearm| frame.now_millis <= rearm)
            || (!pressed
                && (!self.repeat_armed
                    || frame
                        .air_use
                        .is_some_and(|air_use| !air_use.repeats_while_held())))
        {
            return;
        }
        let Some(selection) = self.displayed_selection(frame) else {
            return;
        };
        self.rearm_millis = Some(frame.now_millis.saturating_add(USE_REARM_MILLIS));
        // `baseUseItem` opens a legacy request scope on every air use.
        let legacy_request_id = self.next_legacy_request_id();
        let on_cooldown = frame
            .air_use
            .and_then(AirUse::cooldown)
            .is_some_and(|cooldown| self.on_cooldown(cooldown.category));
        let mut change = None;
        match frame.air_use {
            Some(AirUse::Hold {
                max_ticks,
                slowdown,
                ..
            }) if frame.ready => {
                self.active = Some(ActiveUse {
                    selection: selection.clone(),
                    started_tick: frame.tick,
                    max_ticks,
                    slowdown,
                });
                outcome.started = true;
            }
            Some(AirUse::Throw { cooldown }) if !on_cooldown => {
                outcome.swung = true;
                if let Some(Cooldown { category, ticks }) = cooldown {
                    self.cooldowns
                        .push((category, frame.tick.saturating_add(u64::from(ticks))));
                }
                if !frame.creative {
                    let to = selection.item.less_one(legacy_request_id);
                    self.predicted = frame.selection.as_ref().map(|server| PredictedStack {
                        slot: selection.slot,
                        server: server.item.clone(),
                        stack: to.clone(),
                    });
                    change = Some(PredictedSlotChange {
                        legacy_request_id,
                        from: selection.item.clone(),
                        to,
                    });
                }
            }
            _ => {}
        }
        if let Ok(packet) = protocol::click_air_packet(held_request(&selection, frame), change) {
            outcome.packets.push(packet);
            outcome.used = true;
        }
    }

    /// The selected stack with an unconfirmed throw applied; `None` when nothing is held.
    fn displayed_selection(&mut self, frame: &UseFrame) -> Option<FrozenMiningSelection> {
        let server = frame.selection.as_ref()?;
        let predicted = self
            .predicted
            .as_ref()
            .filter(|predicted| predicted.slot == server.slot && predicted.server == server.item);
        let selection = match predicted {
            Some(predicted) => FrozenMiningSelection {
                slot: server.slot,
                item: predicted.stack.clone(),
            },
            None => {
                self.predicted = None;
                server.clone()
            }
        };
        (!selection.item.is_empty()).then_some(selection)
    }

    fn on_cooldown(&self, category: &str) -> bool {
        self.cooldowns.iter().any(|(active, _)| *active == category)
    }

    /// `TypedClientNetId::_generateNext`: even ids from -4 downward, restarting past the range.
    fn next_legacy_request_id(&mut self) -> i32 {
        let current = if self.last_legacy_request_id < -2 {
            self.last_legacy_request_id
        } else {
            -2
        };
        self.last_legacy_request_id = current.checked_sub(2).unwrap_or(-4);
        self.last_legacy_request_id
    }

    /// The local rig's use flag: set while a use runs, cleared while a held-use item idles.
    pub(crate) fn local_item_use(&self, stream: &WorldStream, ui: &UiRuntime) -> LocalItemUse {
        if self.active.is_some() {
            return LocalItemUse::Using;
        }
        match selected_air_use(stream, ui) {
            Some(AirUse::Hold { .. }) => LocalItemUse::Idle,
            Some(AirUse::Instant | AirUse::Throw { .. }) | None => LocalItemUse::Unpredicted,
        }
    }
}

fn held_request(selection: &FrozenMiningSelection, frame: &UseFrame) -> HeldItemRequest {
    HeldItemRequest {
        selected_slot: selection.slot,
        selected_item: selection.item.clone(),
        player_position: frame.position,
    }
}

/// The selected stack's air use.
pub(crate) fn selected_air_use(stream: &WorldStream, ui: &UiRuntime) -> Option<AirUse> {
    let stack = ui.selected_stack()?;
    let canonical = stream.canonical_item_stack(stack)?;
    let identifier = canonical.identifier.as_deref()?;
    let quick_charge =
        protocol::item_enchantment_level(&stack.extra_data, QUICK_CHARGE_ENCHANTMENT_ID)
            .unwrap_or(0);
    let pack_ticks = stream.item_max_use_ticks(identifier).or_else(|| {
        classify::pack_identifier(identifier).and_then(|pack| stream.item_max_use_ticks(pack))
    });
    classify(
        identifier,
        canonical.charged_projectile.is_some(),
        quick_charge,
        pack_ticks,
    )
}

/// The use duration of the selected stack when vanilla animates its use as eating or drinking.
pub(crate) fn consume_ticks(stream: &WorldStream, ui: &UiRuntime) -> Option<u32> {
    let canonical = stream.canonical_item_stack(ui.selected_stack()?)?;
    match selected_air_use(stream, ui)? {
        AirUse::Hold { max_ticks, .. }
            if classify::is_consumed(canonical.identifier.as_deref()?) =>
        {
            Some(max_ticks)
        }
        _ => None,
    }
}

/// Whether the known state meets `needs`.
fn needs_met(stream: &WorldStream, ui: &UiRuntime, needs: Needs) -> bool {
    let is = |stack: &protocol::NetworkItemStack, identifier: &str| {
        !stack.is_empty()
            && stream
                .item_identifier(stack.network_id)
                .is_some_and(|name| &*name == identifier)
    };
    let arrow_in_inventory = || {
        let ledger = ui.inventory_ledger();
        (0..protocol::PLAYER_INVENTORY_SLOTS)
            .filter_map(|slot| ledger.displayed_stack(slot))
            .chain(ui.gameplay_hud().offhand_stack())
            .any(|stack| is(stack, "minecraft:arrow"))
    };
    match needs {
        Needs::Nothing => true,
        Needs::Arrow => arrow_in_inventory(),
        Needs::ArrowOrOffhandRocket => {
            arrow_in_inventory()
                || ui
                    .gameplay_hud()
                    .offhand_stack()
                    .is_some_and(|stack| is(stack, "minecraft:firework_rocket"))
        }
        // Peaceful difficulty's always-edible rule is not modeled.
        Needs::Appetite => ui
            .hud()
            .hunger()
            .is_none_or(|hunger| hunger.current() < hunger.maximum()),
    }
}

#[derive(SystemParam)]
pub(crate) struct ItemUseContext<'w, 's> {
    input: Res<'w, SemanticInputSnapshot>,
    ui: Res<'w, UiRuntime>,
    menu: Res<'w, MenuRuntime>,
    windows: Query<'w, 's, &'static Window, With<PrimaryWindow>>,
    client_world: Res<'w, ClientWorld>,
    melee: Res<'w, MeleeRuntime>,
    block_use: Res<'w, BlockUseRuntime>,
    effects: Res<'w, LocalMovementEffectTimeline>,
    network: Res<'w, NetworkHandle>,
    time: Res<'w, Time<Real>>,
}

/// Runs after block use so a press that interacted with a block starts no item use.
pub(crate) fn produce_item_use(
    context: ItemUseContext,
    mut runtime: ResMut<ItemUseRuntime>,
    mut movement: ResMut<MovementTicker>,
    mut swings: ResMut<SwingTracker>,
) {
    runtime.synchronize(context.ui.session_id());
    let focused =
        !context.menu.is_visible() && context.windows.single().is_ok_and(|window| window.focused);
    let use_phase = context.input.phase(Action::Use);
    let admitted = if context.input.snapshot().is_none() {
        false
    } else if !focused || context.ui.ui_focused() {
        use_phase
            .pressed
            .then(|| crate::movement::note_click_drop("use", "screen_open"));
        false
    } else if context
        .ui
        .game_mode_capabilities()
        .is_some_and(|caps| !caps.can_use_items)
    {
        use_phase
            .pressed
            .then(|| crate::movement::note_click_drop("use", "spectator"));
        false
    } else {
        true
    };
    if !admitted {
        runtime.cancel_pending_input();
    }
    runtime.observe_press(admitted && use_phase.pressed);
    let held = admitted && use_phase.held;
    let Some(stream) = context.client_world.stream.as_ref() else {
        return;
    };
    if !runtime.has_work(held) {
        return;
    }
    // Frames between physics ticks have no unsent tick; the press waits for one.
    let Some(sample) = movement.newest_unsent_sample() else {
        return;
    };
    let now_millis = u64::try_from(context.time.elapsed().as_millis()).unwrap_or(u64::MAX);
    let air_use = selected_air_use(stream, &context.ui);
    let creative = context.ui.player_game_mode() == Some(PlayerGameMode::Creative);
    let frame = UseFrame {
        tick: sample.tick,
        now_millis,
        position: sample.position,
        held,
        selection: verified_use_selection(&context.ui),
        air_use,
        ready: match air_use {
            Some(AirUse::Hold { needs, .. }) => creative || needs_met(stream, &context.ui, needs),
            _ => false,
        },
        creative,
        press_consumed: context.melee.blocks_use_at(now_millis)
            || context.block_use.interacted_at(sample.tick),
    };
    if let Some(reason) = runtime.press_drop_reason(&frame) {
        crate::movement::note_click_drop("use", reason);
    }
    let duration = swing_duration(context.effects.mining_effects());
    if step_and_send(
        &mut runtime,
        &mut swings,
        &frame,
        stream.local_player_runtime_id(),
        duration,
        |packets| context.network.send_inventory_packets(packets),
    ) {
        movement.mark_started_using_item(sample.tick);
    }
}

#[cfg(test)]
mod tests;
