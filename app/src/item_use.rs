//! Air item use: starting, holding, completing and releasing held-use items.
//!
//! Follows `GameMode::baseUseItem`, `GameMode::releaseUsingItem` and
//! `Player::completeUsingItem`; projectiles, ammunition and damage stay server-owned.

use bevy::{
    ecs::system::SystemParam,
    prelude::{Query, Real, Res, ResMut, Resource, Time, Window, With},
    window::PrimaryWindow,
};
use client_world::{LocalItemUse, WorldStream};
use protocol::{HeldItemRequest, ItemReleaseKind, PlayerGameMode};
use semantic_input::Action;

use crate::{
    block_use::{BlockUseRuntime, verified_use_selection},
    melee::MeleeRuntime,
    menu::MenuRuntime,
    mining::FrozenMiningSelection,
    movement::MovementTicker,
    runtime::{network::NetworkHandle, world::ClientWorld},
    semantic_controls::SemanticInputSnapshot,
    ui_runtime::UiRuntime,
};

/// `BowItem`/`TridentItem::getMaxUseDuration`.
const LONG_USE_TICKS: u32 = 72_000;
const SPYGLASS_USE_TICKS: u32 = 1_200;
/// `CrossbowItem::getMaxUseDuration`: 25 ticks less 5 per Quick Charge level.
const CROSSBOW_CHARGE_TICKS: u32 = 25;
const QUICK_CHARGE_TICKS_PER_LEVEL: u32 = 5;
const QUICK_CHARGE_ENCHANTMENT_ID: i16 = 35;
/// `ItemUseSlowdownSystemImpl`'s movement factor for an item in use without
/// `minecraft:use_modifiers` (0.35, read from the 26.30 client). No handled air
/// use carries that component.
const ITEM_USE_SLOWDOWN: f64 = 0.35;

/// Ammunition a held-use item needs before its use starts outside creative.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Ammo {
    None,
    Arrow,
    /// Arrows anywhere, or a firework rocket in the offhand.
    ArrowOrOffhandRocket,
}

/// What pressing use in the air does with the selected item.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AirUse {
    /// Starts a use that completes after `max_ticks`.
    Hold { max_ticks: u32, ammo: Ammo },
    /// Acts at once, as a loaded crossbow fires.
    Instant,
}

/// The air use of the handled ranged, scoped and thrown items; `None` for everything else.
pub(crate) fn classify(identifier: &str, charged: bool, quick_charge: u8) -> Option<AirUse> {
    let name = identifier.strip_prefix("minecraft:")?;
    Some(match name {
        "bow" => AirUse::Hold {
            max_ticks: LONG_USE_TICKS,
            ammo: Ammo::Arrow,
        },
        "trident" => AirUse::Hold {
            max_ticks: LONG_USE_TICKS,
            ammo: Ammo::None,
        },
        "spyglass" => AirUse::Hold {
            max_ticks: SPYGLASS_USE_TICKS,
            ammo: Ammo::None,
        },
        // Thrown on the press; the server spawns the projectile.
        "ender_pearl" | "snowball" | "egg" | "splash_potion" | "lingering_potion"
        | "experience_bottle" | "wind_charge" | "ender_eye" | "fishing_rod" => AirUse::Instant,
        "crossbow" if charged => AirUse::Instant,
        "crossbow" => AirUse::Hold {
            max_ticks: CROSSBOW_CHARGE_TICKS
                .saturating_sub(u32::from(quick_charge) * QUICK_CHARGE_TICKS_PER_LEVEL),
            ammo: Ammo::ArrowOrOffhandRocket,
        },
        _ => return None,
    })
}

#[derive(Debug, Clone, PartialEq)]
struct ActiveUse {
    selection: FrozenMiningSelection,
    started_tick: u64,
    max_ticks: u32,
}

/// One unsent tick's view of the use input and the selected stack.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct UseFrame {
    pub(crate) tick: u64,
    pub(crate) position: [f32; 3],
    pub(crate) held: bool,
    pub(crate) selection: Option<FrozenMiningSelection>,
    pub(crate) air_use: Option<AirUse>,
    pub(crate) has_ammo: bool,
    /// A block interaction or recent attack consumed this press.
    pub(crate) press_consumed: bool,
}

/// Transactions in send order, plus whether a use began on this tick.
#[derive(Debug, Default)]
pub(crate) struct UseOutcome {
    pub(crate) packets: Vec<protocol::Packet>,
    pub(crate) started: bool,
}

/// The press latch and the accepted item use.
#[derive(Resource, Debug, Default)]
pub(crate) struct ItemUseRuntime {
    latched_press: bool,
    active: Option<ActiveUse>,
    session: Option<u64>,
}

impl ItemUseRuntime {
    /// Whether a use started locally and has not ended.
    pub(crate) const fn is_using(&self) -> bool {
        self.active.is_some()
    }

    /// Movement-input factor while a use runs; `None` when idle.
    pub(crate) fn movement_modifier(&self) -> Option<f64> {
        self.active.as_ref().map(|_| ITEM_USE_SLOWDOWN)
    }

    /// A new session drops the press and any use without packets.
    pub(crate) fn synchronize(&mut self, session: u64) {
        if self.session.is_some_and(|previous| previous != session) {
            self.latched_press = false;
            self.active = None;
        }
        self.session = Some(session);
    }

    pub(crate) fn observe_press(&mut self, pressed: bool) {
        self.latched_press |= pressed;
    }

    /// Whether this frame has anything to resolve against an unsent tick.
    pub(crate) const fn has_work(&self) -> bool {
        self.latched_press || self.active.is_some()
    }

    /// Resolves a latched press, then ends the use on release, depletion or reselection.
    pub(crate) fn step(&mut self, frame: &UseFrame) -> UseOutcome {
        let mut outcome = UseOutcome::default();
        if std::mem::take(&mut self.latched_press) && self.active.is_none() {
            self.start(frame, &mut outcome);
        }
        let Some(active) = &self.active else {
            return outcome;
        };
        let selection = match &frame.selection {
            Some(current)
                if current.slot != active.selection.slot
                    || current.item.network_id() != active.selection.item.network_id() =>
            {
                // Switching away stops the use without a release, as `Player::stopUsingItem`.
                self.active = None;
                return outcome;
            }
            Some(current) => current.clone(),
            // An in-flight inventory request hides the stack; keep the one the use began with.
            None => active.selection.clone(),
        };
        let kind = if !frame.held {
            ItemReleaseKind::Release
        } else if frame.tick.saturating_sub(active.started_tick) >= u64::from(active.max_ticks) {
            ItemReleaseKind::Complete
        } else {
            return outcome;
        };
        self.active = None;
        if let Ok(packet) = protocol::release_item_packet(held_request(&selection, frame), kind) {
            outcome.packets.push(packet);
        }
        outcome
    }

    fn start(&mut self, frame: &UseFrame, outcome: &mut UseOutcome) {
        let (Some(selection), Some(air_use)) = (&frame.selection, frame.air_use) else {
            return;
        };
        if frame.press_consumed {
            return;
        }
        if let Ok(packet) = protocol::click_air_packet(held_request(selection, frame)) {
            outcome.packets.push(packet);
        }
        if let AirUse::Hold { max_ticks, .. } = air_use
            && frame.has_ammo
        {
            self.active = Some(ActiveUse {
                selection: selection.clone(),
                started_tick: frame.tick,
                max_ticks,
            });
            outcome.started = true;
        }
    }

    /// The local rig's use flag: set while a use runs, cleared while a handled item idles.
    pub(crate) fn local_item_use(&self, stream: &WorldStream, ui: &UiRuntime) -> LocalItemUse {
        if self.active.is_some() {
            return LocalItemUse::Using;
        }
        match selected_air_use(stream, ui) {
            Some(AirUse::Hold { .. }) => LocalItemUse::Idle,
            Some(AirUse::Instant) | None => LocalItemUse::Unpredicted,
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
    let quick_charge =
        protocol::item_enchantment_level(&stack.extra_data, QUICK_CHARGE_ENCHANTMENT_ID)
            .unwrap_or(0);
    classify(
        canonical.identifier.as_deref()?,
        canonical.charged_projectile.is_some(),
        quick_charge,
    )
}

/// Whether the known inventory holds the ammunition `ammo` needs.
fn has_ammo(stream: &WorldStream, ui: &UiRuntime, ammo: Ammo) -> bool {
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
    match ammo {
        Ammo::None => true,
        Ammo::Arrow => arrow_in_inventory(),
        Ammo::ArrowOrOffhandRocket => {
            arrow_in_inventory()
                || ui
                    .gameplay_hud()
                    .offhand_stack()
                    .is_some_and(|stack| is(stack, "minecraft:firework_rocket"))
        }
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
    network: Res<'w, NetworkHandle>,
    time: Res<'w, Time<Real>>,
}

/// Runs after block use so a press that interacted with a block starts no item use.
pub(crate) fn produce_item_use(
    context: ItemUseContext,
    mut runtime: ResMut<ItemUseRuntime>,
    mut movement: ResMut<MovementTicker>,
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
    runtime.observe_press(admitted && use_phase.pressed);
    let Some(stream) = context.client_world.stream.as_ref() else {
        return;
    };
    if !runtime.has_work() {
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
        position: sample.position,
        held: admitted && use_phase.held,
        selection: verified_use_selection(&context.ui),
        air_use,
        has_ammo: match air_use {
            Some(AirUse::Hold { ammo, .. }) => creative || has_ammo(stream, &context.ui, ammo),
            _ => false,
        },
        press_consumed: context.melee.blocks_use_at(now_millis)
            || context.block_use.interacted_at(sample.tick),
    };
    if runtime.latched_press && !runtime.is_using() {
        let reason = if frame.selection.is_none() {
            Some("selection_unverified")
        } else if frame.air_use.is_none() {
            Some("no_air_use_for_item")
        } else if frame.press_consumed {
            Some("consumed_by_block_or_attack")
        } else {
            None
        };
        if let Some(reason) = reason {
            crate::movement::note_click_drop("use", reason);
        }
    }
    let outcome = runtime.step(&frame);
    for packet in outcome.packets {
        let _ = context.network.send_inventory_packet(packet);
    }
    if outcome.started {
        movement.mark_started_using_item(sample.tick);
    }
}

#[cfg(test)]
mod tests;
