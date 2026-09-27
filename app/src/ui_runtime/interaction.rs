use bevy::{
    input::{
        ButtonState,
        gamepad::{Gamepad, GamepadButton},
        keyboard::KeyboardInput,
        mouse::AccumulatedMouseMotion,
        touch::Touches,
    },
    math::Vec2,
    prelude::{
        ButtonInput, KeyCode, MessageReader, MouseButton, Query, Res, ResMut, Single, Time, With,
    },
    time::Real,
    window::{CursorGrabMode, CursorOptions, PrimaryWindow, Window},
};

use crate::acceptance::markers::FAST_TRANSFER_ACTION;
use protocol::{ChatPacketError, Packet};
use ui::{ChatClipboard, ChatEditor, PointerPhase, UiAction, UiPoint};

use super::inventory_ledger::{CellGesture, DropSource, InventoryGestureError, InventoryTarget};
use super::{PlatformClipboard, UiRuntime, presentation};
use presentation::inventory_pointer::InventoryCellHit;

/// Admits every ready inventory packet in queue order, stopping at the first
/// transport refusal. Returns whether anything was admitted.
pub fn flush_inventory_send<E>(
    runtime: &mut UiRuntime,
    now_millis: u64,
    mut send: impl FnMut(Packet) -> Result<(), E>,
) -> Result<bool, E> {
    runtime.poll_inventory_timeout(now_millis);
    let mut admitted_any = false;
    for _ in 0..MAX_INVENTORY_PACKETS_PER_FLUSH {
        let Some(packet) = runtime
            .inventory_ledger()
            .pending_packet()
            .expect("the ledger retains only validated protocol requests")
        else {
            break;
        };
        if let Err(error) = send(packet) {
            runtime
                .inventory_ledger_mut()
                .note_transport_pressure(now_millis);
            return Err(error);
        }
        let admitted = runtime
            .inventory_ledger_mut()
            .mark_transport_enqueued(now_millis);
        debug_assert!(admitted, "only an awaiting request can be transported");
        admitted_any = true;
    }
    Ok(admitted_any)
}

/// Bounds one frame's inventory transport work.
const MAX_INVENTORY_PACKETS_PER_FLUSH: usize = 32;

pub(crate) fn flush_inventory_network(
    time: Res<Time<Real>>,
    mut runtime: ResMut<UiRuntime>,
    network: Res<crate::runtime::network::NetworkHandle>,
) {
    let now_millis = u64::try_from(time.elapsed().as_millis()).unwrap_or(u64::MAX);
    match flush_inventory_send(&mut runtime, now_millis, |packet| {
        network.send_inventory_packet(packet)
    }) {
        Ok(_) | Err(crate::runtime::network::PacketSendError::Full(_)) => {}
        Err(crate::runtime::network::PacketSendError::Closed(_)) => {
            runtime.inventory_transport_closed();
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum ChatFlushError<E> {
    Packet(ChatPacketError),
    Transport(E),
    SessionChanged { expected: u64, actual: u64 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FastTransferAction {
    TransferSm3,
}

impl FastTransferAction {
    fn classify(message: &str) -> Option<Self> {
        (message == "/transfer sm3").then_some(Self::TransferSm3)
    }

    pub(crate) fn marker(
        self,
        session_generation: u64,
        action_ordinal: u64,
        sent_unix_ms: u64,
    ) -> String {
        let command = match self {
            Self::TransferSm3 => "/transfer sm3",
        };
        format!(
            "{FAST_TRANSFER_ACTION}={}",
            serde_json::json!({
                "schema": "rust-mcbe-fast-transfer-action-v1",
                "kind": "command_sent",
                "session_generation": session_generation,
                "action_ordinal": action_ordinal,
                "command": command,
                "sent_unix_ms": sent_unix_ms,
            })
        )
    }
}

pub fn flush_chat_sends<E>(
    runtime: &mut UiRuntime,
    budget: usize,
    mut send: impl FnMut(u64, u64, Option<FastTransferAction>, Packet) -> Result<(), E>,
) -> Result<usize, ChatFlushError<E>> {
    if budget == 0 || runtime.in_flight_chat_send().is_some() {
        return Ok(0);
    }
    let mut sent = 0;
    for _ in 0..budget.min(1) {
        let Some(request) = runtime.pending_chat_sends().front() else {
            break;
        };
        if request.session != runtime.session_id() {
            return Err(ChatFlushError::SessionChanged {
                expected: runtime.session_id(),
                actual: request.session,
            });
        }
        let (sequence, packet) = runtime
            .front_chat_packet()
            .map_err(ChatFlushError::Packet)?
            .expect("the pending front was observed above");
        send(
            request.session,
            sequence,
            FastTransferAction::classify(&request.message),
            packet,
        )
        .map_err(ChatFlushError::Transport)?;
        let enqueued = runtime.mark_chat_send_enqueued(request.session, sequence);
        debug_assert!(
            enqueued,
            "only the observed FIFO front can become in flight"
        );
        sent += 1;
    }
    Ok(sent)
}

pub(crate) fn flush_chat_network(
    mut runtime: ResMut<UiRuntime>,
    network: Res<crate::runtime::network::NetworkHandle>,
    mut client_world: ResMut<crate::runtime::world::ClientWorld>,
) {
    runtime.service_pending_chat_autocomplete();
    if network.closed_command_has_pending_control() {
        return;
    }
    match flush_chat_sends(
        &mut runtime,
        8,
        |session, sequence, action, packet| match network
            .send_chat_packet(session, sequence, action, packet)
        {
            Err(crate::runtime::network::PacketSendError::Closed(packet))
                if network.closed_command_has_pending_control() =>
            {
                Err(crate::runtime::network::PacketSendError::Full(packet))
            }
            result => result,
        },
    ) {
        Ok(_)
        | Err(ChatFlushError::Transport(crate::runtime::network::PacketSendError::Full(_))) => {}
        Err(ChatFlushError::Transport(crate::runtime::network::PacketSendError::Closed(_))) => {
            crate::runtime::shutdown::record_fatal_error(
                &mut client_world.fatal_error,
                "chat send failed because the network command channel closed".to_owned(),
            );
        }
        Err(ChatFlushError::Packet(error)) => {
            crate::runtime::shutdown::record_fatal_error(
                &mut client_world.fatal_error,
                format!("queued chat packet became invalid: {error}"),
            );
        }
        Err(ChatFlushError::SessionChanged { expected, actual }) => {
            crate::runtime::shutdown::record_fatal_error(
                &mut client_world.fatal_error,
                format!(
                    "queued chat packet crossed a session boundary: expected {expected}, got {actual}"
                ),
            );
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn drive_chat_ui_actions(
    time: Res<Time<Real>>,
    window: Single<&Window, With<PrimaryWindow>>,
    menu: Option<Res<crate::menu::MenuRuntime>>,
    mouse_buttons: Res<ButtonInput<MouseButton>>,
    touches: Res<Touches>,
    gamepads: Query<&Gamepad>,
    presentation: Res<presentation::UiPresentationRuntime>,
    mut runtime: ResMut<UiRuntime>,
) {
    if runtime.server_forms().owns_input() {
        return;
    }
    if menu.as_ref().is_some_and(|menu| menu.is_visible())
        || !runtime.chat_focused()
        || !window.focused
    {
        return;
    }
    let logical_size = [window.width(), window.height()];
    let now_millis = u64::try_from(time.elapsed().as_millis()).unwrap_or(u64::MAX);

    if mouse_buttons.just_pressed(MouseButton::Left)
        && let Some(position) = window.cursor_position()
        && let Ok(position) = UiPoint::new(position.x, position.y)
    {
        dispatch_chat_ui_action(
            &mut runtime,
            UiAction::PointerPrimary {
                position,
                phase: PointerPhase::Pressed,
            },
            presentation.hit_test_chat_suggestion(position, logical_size),
            now_millis,
        );
    }
    for touch in touches.iter_just_pressed() {
        let position = touch.position();
        if let Ok(position) = UiPoint::new(position.x, position.y) {
            dispatch_chat_ui_action(
                &mut runtime,
                UiAction::PointerPrimary {
                    position,
                    phase: PointerPhase::Pressed,
                },
                presentation.hit_test_chat_suggestion(position, logical_size),
                now_millis,
            );
        }
    }
    for gamepad in &gamepads {
        for button in [
            GamepadButton::DPadUp,
            GamepadButton::DPadDown,
            GamepadButton::South,
            GamepadButton::East,
            GamepadButton::RightTrigger,
            GamepadButton::LeftTrigger,
        ] {
            if gamepad.just_pressed(button) {
                dispatch_chat_ui_action(
                    &mut runtime,
                    gamepad_chat_action(button).expect("the mapped button list is exhaustive"),
                    None,
                    now_millis,
                );
            }
        }
    }
}

/// Inventory keyboard input captured before gameplay suppression resets the
/// frame's key state: this frame's presses and the held modifiers.
#[derive(Debug, Clone, Default)]
pub(crate) struct InventoryKeys {
    presses: Vec<KeyCode>,
    shift: bool,
    control: bool,
}

impl InventoryKeys {
    /// Bounds one frame's buffered presses.
    const MAX_PRESSES: usize = 16;

    fn track_modifier(&mut self, input: &KeyboardInput) {
        let pressed = input.state == ButtonState::Pressed;
        match input.key_code {
            KeyCode::ShiftLeft | KeyCode::ShiftRight => self.shift = pressed,
            KeyCode::ControlLeft | KeyCode::ControlRight => self.control = pressed,
            _ => {}
        }
    }

    fn press(&mut self, key: KeyCode) {
        if self.presses.len() < Self::MAX_PRESSES {
            self.presses.push(key);
        }
    }
}

pub(crate) fn drive_inventory_ui_actions(
    window: Single<&Window, With<PrimaryWindow>>,
    menu: Option<Res<crate::menu::MenuRuntime>>,
    mut mouse_buttons: ResMut<ButtonInput<MouseButton>>,
    presentation: Res<presentation::UiPresentationRuntime>,
    mut runtime: ResMut<UiRuntime>,
) {
    // Presses are this frame's only; modifiers stay held across frames.
    let presses = std::mem::take(&mut runtime.inventory_keys.presses);
    let (shift, control) = (runtime.inventory_keys.shift, runtime.inventory_keys.control);
    if runtime.server_forms().owns_input() {
        return;
    }
    if menu.as_ref().is_some_and(|menu| menu.is_visible())
        || !runtime.inventory_open()
        || !window.focused
    {
        runtime.set_inventory_pointer_gui(None);
        return;
    }
    let primary_pressed = mouse_buttons.just_pressed(MouseButton::Left);
    let secondary_pressed = mouse_buttons.just_pressed(MouseButton::Right);
    // The inventory owns pointer buttons while open. Preserve the primary
    // and secondary edges long enough to resolve their cell, then clear every
    // button before gameplay systems can observe this frame.
    mouse_buttons.reset_all();
    let Some(position) = window.cursor_position() else {
        runtime.set_inventory_pointer_gui(None);
        return;
    };
    let Ok(point) = UiPoint::new(position.x, position.y) else {
        runtime.set_inventory_pointer_gui(None);
        return;
    };
    let physical_size = [window.physical_width(), window.physical_height()];
    let gui = presentation.inventory_gui_point(point, physical_size, window.scale_factor());
    runtime.set_inventory_pointer_gui(gui);
    let screen = presentation::inventory_pointer::InventoryScreen::of(runtime.inventory_ledger());
    let hit = gui.and_then(|gui| {
        presentation.inventory_cell_hit(gui, physical_size, window.scale_factor(), screen)
    });
    for key in presses {
        let _ = dispatch_inventory_key(runtime.as_mut(), hit, key, control);
    }
    let Some(hit) = hit else {
        // A held stack released outside the panel is dropped: all of it on a
        // primary click, one item on a secondary click.
        let outside = gui.is_some_and(|gui| {
            !presentation.inventory_panel_contains(
                gui,
                physical_size,
                window.scale_factor(),
                screen,
            )
        });
        if outside && (primary_pressed || secondary_pressed) {
            let amount = (!primary_pressed).then_some(1);
            let _ = runtime
                .inventory_ledger_mut()
                .begin_drop(DropSource::Cursor, amount);
        }
        return;
    };
    // When both physical edges arrive together, the primary operation wins
    // as a deterministic local policy.
    if primary_pressed
        && shift
        && let Some(target) = gesture_target(hit)
    {
        let _ = runtime.inventory_ledger_mut().begin_quick_move(target);
    } else if primary_pressed {
        let _ = dispatch_inventory_click(runtime.as_mut(), hit, CellGesture::Click);
    } else if secondary_pressed {
        let ledger = runtime.inventory_ledger();
        let Some(target) = gesture_target(hit) else {
            return;
        };
        let gesture = match (ledger.cursor_stack(), ledger.target_stack(target)) {
            (Some(_), _) => CellGesture::PlaceCount(1),
            (None, Some(stack)) => CellGesture::TakeCount(stack.count.div_ceil(2)),
            (None, None) => return,
        };
        let _ = dispatch_inventory_click(runtime.as_mut(), hit, gesture);
    }
}

const fn gesture_target(hit: InventoryCellHit) -> Option<InventoryTarget> {
    Some(match hit {
        InventoryCellHit::Player(slot) => InventoryTarget::Player(slot),
        InventoryCellHit::Storage(slot) => InventoryTarget::Storage(slot),
        InventoryCellHit::Armor(slot) => InventoryTarget::Armor(slot),
        InventoryCellHit::Offhand => InventoryTarget::Offhand,
        InventoryCellHit::Craft(slot) => InventoryTarget::Craft(slot),
        InventoryCellHit::CraftOutput => return None,
    })
}

/// Keyboard gestures over the hovered cell: digits swap with that hotbar
/// cell, Q drops one item and Control+Q the whole stack.
pub(crate) fn dispatch_inventory_key(
    runtime: &mut UiRuntime,
    hit: Option<InventoryCellHit>,
    key: KeyCode,
    control: bool,
) -> Option<Result<i32, InventoryGestureError>> {
    let target = gesture_target(hit?)?;
    let ledger = runtime.inventory_ledger_mut();
    let hotbar = [
        KeyCode::Digit1,
        KeyCode::Digit2,
        KeyCode::Digit3,
        KeyCode::Digit4,
        KeyCode::Digit5,
        KeyCode::Digit6,
        KeyCode::Digit7,
        KeyCode::Digit8,
        KeyCode::Digit9,
    ]
    .iter()
    .position(|digit| *digit == key);
    match (hotbar, key) {
        (Some(slot), _) => Some(ledger.begin_hotbar_swap(target, slot as u8)),
        (None, KeyCode::KeyQ) => {
            let amount = (!control).then_some(1);
            Some(ledger.begin_drop(DropSource::Target(target), amount))
        }
        _ => None,
    }
}

/// Routes one resolved pointer gesture; the output cell crafts once.
pub(crate) fn dispatch_inventory_click(
    runtime: &mut UiRuntime,
    hit: InventoryCellHit,
    gesture: CellGesture,
) -> Result<i32, InventoryGestureError> {
    match gesture_target(hit) {
        Some(target) => runtime
            .inventory_ledger_mut()
            .begin_target_gesture(target, gesture),
        None if gesture == CellGesture::Click => runtime.begin_crafting(),
        None => Err(InventoryGestureError::InvalidRequest),
    }
}

pub(crate) const fn gamepad_chat_action(button: GamepadButton) -> Option<UiAction> {
    match button {
        GamepadButton::DPadUp => Some(UiAction::Navigate([0, -1])),
        GamepadButton::DPadDown => Some(UiAction::Navigate([0, 1])),
        GamepadButton::South => Some(UiAction::Accept),
        GamepadButton::East => Some(UiAction::Cancel),
        GamepadButton::RightTrigger => Some(UiAction::TabNext),
        GamepadButton::LeftTrigger => Some(UiAction::TabPrevious),
        _ => None,
    }
}

pub(crate) fn dispatch_chat_ui_action(
    runtime: &mut UiRuntime,
    action: UiAction,
    suggestion_hit: Option<usize>,
    now_millis: u64,
) -> bool {
    match action {
        UiAction::Cancel => {
            runtime.close_chat();
            true
        }
        UiAction::Accept if runtime.chat_suggestions().is_empty() => {
            if runtime.queue_chat_send(now_millis).is_err() {
                return false;
            }
            runtime.close_chat();
            true
        }
        _ => runtime.handle_chat_ui_action_with_suggestion_hit(action, suggestion_hit),
    }
}

fn is_chat_paste_shortcut(key: KeyCode, keys: &ButtonInput<KeyCode>) -> bool {
    key == KeyCode::KeyV
        && (keys.pressed(KeyCode::ControlLeft)
            || keys.pressed(KeyCode::ControlRight)
            || keys.pressed(KeyCode::SuperLeft)
            || keys.pressed(KeyCode::SuperRight))
        && !keys.pressed(KeyCode::AltLeft)
        && !keys.pressed(KeyCode::AltRight)
}

pub(crate) fn paste_chat_shortcut<C: ChatClipboard>(
    runtime: &mut UiRuntime,
    key: KeyCode,
    keys: &ButtonInput<KeyCode>,
    clipboard: &mut C,
) -> bool {
    if !is_chat_paste_shortcut(key, keys) {
        return false;
    }
    let _ = runtime.paste_chat_text(clipboard);
    true
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn drive_chat_keyboard_input(
    mut keyboard_messages: MessageReader<KeyboardInput>,
    time: Res<Time<Real>>,
    window: Single<(&Window, &mut CursorOptions), With<PrimaryWindow>>,
    menu: Option<Res<crate::menu::MenuRuntime>>,
    mut keys: ResMut<ButtonInput<KeyCode>>,
    mut mouse_buttons: ResMut<ButtonInput<MouseButton>>,
    mut mouse_motion: ResMut<AccumulatedMouseMotion>,
    mut runtime: ResMut<UiRuntime>,
) {
    let (window, mut cursor) = window.into_inner();
    if runtime.server_forms().owns_input() {
        keyboard_messages.clear();
        return;
    }
    if menu.as_ref().is_some_and(|menu| menu.is_visible()) {
        if runtime.inventory_open() {
            runtime.close_inventory();
        }
        keyboard_messages.clear();
        // The menu system runs next and must see the original button state.
        // It consumes keyboard/pointer input after handling its own actions.
        mouse_motion.delta = Vec2::ZERO;
        cursor.grab_mode = CursorGrabMode::None;
        cursor.visible = true;
        return;
    }
    if !window.focused {
        if runtime.chat_focused() {
            runtime.close_chat();
        }
        if runtime.inventory_open() {
            runtime.close_inventory();
        }
        return;
    }

    // An already-open inventory owns this frame's pointer edge. Keyboard
    // transitions below may close it or open a new UI, so both sides of the
    // transition are checked before preserving that edge for the inventory
    // system later in the production chain.
    let inventory_owned_pointer = runtime.inventory_open();
    let mut inventory_ownership_changed = false;
    let mut consumed_gameplay = runtime.ui_focused();
    for input in keyboard_messages.read() {
        runtime.inventory_keys.track_modifier(input);
        if input.state != ButtonState::Pressed {
            continue;
        }
        if runtime.inventory_open() {
            consumed_gameplay = true;
            match input.key_code {
                KeyCode::KeyE => {
                    runtime.toggle_inventory();
                    inventory_ownership_changed = true;
                }
                KeyCode::Escape => {
                    runtime.close_inventory();
                    inventory_ownership_changed = true;
                }
                key => runtime.inventory_keys.press(key),
            }
            continue;
        }
        if !runtime.chat_focused() {
            match input.key_code {
                KeyCode::KeyE => {
                    runtime.toggle_inventory();
                    inventory_ownership_changed = true;
                    consumed_gameplay = true;
                }
                KeyCode::KeyT => {
                    runtime.open_chat();
                    consumed_gameplay = true;
                }
                KeyCode::Slash => {
                    runtime.open_chat();
                    let _ = runtime.insert_chat_text("/");
                    consumed_gameplay = true;
                }
                _ => {}
            }
            continue;
        }

        consumed_gameplay = true;
        let selecting = keys.pressed(KeyCode::ShiftLeft) || keys.pressed(KeyCode::ShiftRight);
        if paste_chat_shortcut(&mut runtime, input.key_code, &keys, &mut PlatformClipboard) {
            continue;
        }
        match input.key_code {
            KeyCode::Escape => {
                runtime.close_chat();
            }
            KeyCode::Enter | KeyCode::NumpadEnter => {
                if runtime.chat_suggestions().is_empty() {
                    let now_millis = u64::try_from(time.elapsed().as_millis()).unwrap_or(u64::MAX);
                    if runtime.queue_chat_send(now_millis).is_ok() {
                        runtime.close_chat();
                    }
                } else {
                    runtime.handle_chat_ui_action(UiAction::Accept);
                }
            }
            KeyCode::Backspace => runtime.backspace_chat_text(),
            KeyCode::Delete => runtime.delete_chat_text(),
            KeyCode::ArrowLeft => {
                if selecting {
                    runtime.mutate_chat_editor(ChatEditor::select_left);
                } else {
                    runtime.move_chat_cursor_left();
                }
            }
            KeyCode::ArrowRight => {
                if selecting {
                    runtime.mutate_chat_editor(ChatEditor::select_right);
                } else {
                    runtime.move_chat_cursor_right();
                }
            }
            KeyCode::Home => runtime.move_chat_cursor_home(selecting),
            KeyCode::End => runtime.move_chat_cursor_end(selecting),
            KeyCode::ArrowUp => {
                if runtime.chat_suggestions().is_empty() {
                    runtime.show_older_chat_history();
                } else {
                    runtime.handle_chat_ui_action(UiAction::Navigate([0, -1]));
                }
            }
            KeyCode::ArrowDown => {
                if runtime.chat_suggestions().is_empty() {
                    runtime.show_newer_chat_history();
                } else {
                    runtime.handle_chat_ui_action(UiAction::Navigate([0, 1]));
                }
            }
            KeyCode::Tab => {
                runtime.handle_chat_ui_action(if selecting {
                    UiAction::TabPrevious
                } else {
                    UiAction::TabNext
                });
            }
            _ => {
                let modified = keys.pressed(KeyCode::ControlLeft)
                    || keys.pressed(KeyCode::ControlRight)
                    || keys.pressed(KeyCode::AltLeft)
                    || keys.pressed(KeyCode::AltRight)
                    || keys.pressed(KeyCode::SuperLeft)
                    || keys.pressed(KeyCode::SuperRight);
                if !modified
                    && let Some(text) = input.text.as_deref()
                    && !text.chars().any(char::is_control)
                {
                    let _ = runtime.insert_chat_text(text);
                }
            }
        }
    }

    if consumed_gameplay {
        if inventory_owned_pointer && !inventory_ownership_changed && runtime.inventory_open() {
            suppress_gameplay_input_for_inventory(
                &runtime,
                &mut cursor,
                &mut keys,
                &mut mouse_motion,
            );
        } else {
            suppress_gameplay_input_for_chat(
                &runtime,
                &mut cursor,
                &mut keys,
                &mut mouse_buttons,
                &mut mouse_motion,
            );
        }
        // A send/cancel closes chat before suppression, but that same physical
        // key must still be consumed for the current frame.
        if !runtime.ui_focused() {
            restore_gameplay_input_after_chat(
                &mut cursor,
                &mut keys,
                &mut mouse_buttons,
                &mut mouse_motion,
            );
        }
    }
}

fn suppress_gameplay_input_for_inventory(
    runtime: &UiRuntime,
    cursor: &mut CursorOptions,
    keys: &mut ButtonInput<KeyCode>,
    mouse_motion: &mut AccumulatedMouseMotion,
) {
    if !runtime.inventory_open() {
        return;
    }
    cursor.grab_mode = CursorGrabMode::None;
    cursor.visible = true;
    keys.reset_all();
    mouse_motion.delta = Vec2::ZERO;
}

pub(crate) fn restore_gameplay_input_after_chat(
    cursor: &mut CursorOptions,
    keys: &mut ButtonInput<KeyCode>,
    mouse_buttons: &mut ButtonInput<MouseButton>,
    mouse_motion: &mut AccumulatedMouseMotion,
) {
    cursor.grab_mode = CursorGrabMode::Locked;
    cursor.visible = false;
    keys.reset_all();
    mouse_buttons.reset_all();
    mouse_motion.delta = bevy::math::Vec2::ZERO;
}

pub(crate) fn suppress_gameplay_input_for_chat(
    runtime: &UiRuntime,
    cursor: &mut CursorOptions,
    keys: &mut ButtonInput<KeyCode>,
    mouse_buttons: &mut ButtonInput<MouseButton>,
    mouse_motion: &mut AccumulatedMouseMotion,
) {
    if !runtime.ui_focused() {
        return;
    }
    cursor.grab_mode = CursorGrabMode::None;
    cursor.visible = true;
    keys.reset_all();
    mouse_buttons.reset_all();
    mouse_motion.delta = bevy::math::Vec2::ZERO;
}
