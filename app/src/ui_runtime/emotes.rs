//! Physical input and preference handoff for the native JSON-UI emote wheel.
mod controls;

use bevy::{
    ecs::system::SystemParam,
    input::{
        ButtonState,
        gamepad::{Gamepad, GamepadButton},
        keyboard::KeyboardInput,
        mouse::AccumulatedMouseMotion,
    },
    prelude::*,
    time::Real,
    window::{CursorGrabMode, CursorOptions, PrimaryWindow},
};
use client_ui::ui_runtime::presentation::forms::EmoteHit;
use launcher::menu::settings_options::EMOTE_SLOT_COUNT;
use semantic_input::Action;
use ui::{UiAction, UiPoint};

use super::{UiRuntime, presentation::UiPresentationRuntime};
use crate::{
    menu::{
        MenuRuntime,
        settings_options::{binding_gamepad, binding_key, binding_mouse, gamepad_button},
    },
    player_runtime::PlayerRuntime,
    runtime::world::ClientWorld,
    semantic_controls::SemanticInputSnapshot,
};

type SlotPreferences = [Option<String>; EMOTE_SLOT_COUNT];

#[derive(Default)]
pub(crate) struct ObservedEmotes {
    identity: Option<(u64, u64, i32, u64)>,
    preferences: Option<Option<SlotPreferences>>,
    pointer: Option<UiPoint>,
}

/// Keeps a closing wheel's physical input from becoming chat or inventory input.
#[derive(Resource, Default)]
pub(crate) struct EmoteInputConsumed(pub(crate) bool);

#[derive(SystemParam)]
pub(crate) struct EmoteInput<'w, 's> {
    time: Res<'w, Time<Real>>,
    window: Single<'w, 's, (&'static Window, &'static mut CursorOptions), With<PrimaryWindow>>,
    keys: ResMut<'w, ButtonInput<KeyCode>>,
    mouse: ResMut<'w, ButtonInput<MouseButton>>,
    motion: ResMut<'w, AccumulatedMouseMotion>,
    keyboard: MessageReader<'w, 's, KeyboardInput>,
    pads: Query<'w, 's, &'static Gamepad>,
    menu: Option<ResMut<'w, MenuRuntime>>,
    consent: Option<Res<'w, crate::server_experiences::input::ConsentInput>>,
    world: Option<Res<'w, ClientWorld>>,
    player: Res<'w, PlayerRuntime>,
    runtime: ResMut<'w, UiRuntime>,
    presentation: Option<ResMut<'w, UiPresentationRuntime>>,
    observed: Local<'s, ObservedEmotes>,
    consumed: ResMut<'w, EmoteInputConsumed>,
}

/// Runs before chat/menu adapters and before semantic gameplay is finalized.
pub(crate) fn drive_emote_input(mut input: EmoteInput) {
    input.consumed.0 = false;
    let mut owned = input.runtime.emotes().is_open();
    let runtime_id = input.runtime.local_runtime_id(&input.player);
    let stream = input
        .world
        .as_deref()
        .and_then(|world| world.stream.as_ref());
    let rig = runtime_id.and_then(|id| stream.and_then(|stream| stream.actor_rig(id)));
    let identity = runtime_id.map(|id| {
        (
            input.runtime.session_id(),
            id,
            stream.map_or(0, |stream| stream.current_dimension()),
            rig.map_or(0, |rig| rig.reset_generation),
        )
    });
    if identity != input.observed.identity {
        input.runtime.emotes_mut().close();
        input.runtime.emotes_mut().stop();
        input.observed.identity = identity;
        input.observed.preferences = None;
        input.observed.pointer = None;
    }
    if let Some(menu) = input.menu.as_deref() {
        let preferences = menu.settings_snapshot().0.emote_slots().cloned();
        if input.observed.preferences.as_ref() != Some(&preferences) {
            input
                .runtime
                .emotes_mut()
                .apply_preferences(preferences.as_ref());
            input.observed.preferences = Some(preferences);
        }
    }
    let actor_unavailable = runtime_id.is_none()
        || (input.world.is_some() && stream.is_none())
        || runtime_id
            .and_then(|id| stream.and_then(|stream| stream.actor(id)))
            .is_some_and(|actor| actor.status.dead);
    let blocked = input.consent.as_ref().is_some_and(|consent| consent.0)
        || !input.window.0.focused
        || input.menu.as_ref().is_some_and(|menu| menu.is_visible())
        || input.runtime.chat_focused()
        || input.runtime.inventory_open()
        || input.runtime.server_forms().owns_input()
        || input.runtime.sign_editor().active().is_some()
        || input.runtime.local_sleeping()
        || actor_unavailable;
    if blocked {
        input.keyboard.clear();
        input.runtime.emotes_mut().close();
        input.runtime.emotes_mut().stop();
        if let Some(presentation) = input.presentation.as_deref_mut() {
            presentation.set_emote_pointer(None);
        }
        return;
    }
    let now = u64::try_from(input.time.elapsed().as_millis()).unwrap_or(u64::MAX);
    let pointer = input
        .window
        .0
        .cursor_position()
        .and_then(|position| UiPoint::new(position.x, position.y).ok());
    let mouse_toggled = binding_mouse(input.menu.as_deref(), "key.emote", &input.mouse);
    // An open wheel owns D-pad selection, including the default opening button.
    let navigating =
        input.runtime.emotes().is_open() && controls::directional_navigation(&input.pads);
    let gamepad_toggled =
        !navigating && binding_gamepad(input.menu.as_deref(), "key.emote", &input.pads);
    let toggled = mouse_toggled || gamepad_toggled;
    if toggled {
        if let Some(presentation) = input.presentation.as_deref_mut() {
            presentation.set_emote_input_mode(if gamepad_toggled {
                json_ui::InputMode::Gamepad
            } else {
                json_ui::InputMode::Mouse
            });
        }
        toggle(&mut input.runtime);
        owned = true;
    }
    for event in input.keyboard.read() {
        if event.state != ButtonState::Pressed || event.repeat {
            continue;
        }
        if binding_key(input.menu.as_deref(), "key.emote", event.key_code) {
            if let Some(presentation) = input.presentation.as_deref_mut() {
                presentation.set_emote_input_mode(json_ui::InputMode::Mouse);
            }
            toggle(&mut input.runtime);
            owned = true;
            continue;
        }
        if !input.runtime.emotes().is_open() {
            continue;
        }
        owned = true;
        if let Some(presentation) = input.presentation.as_deref_mut() {
            presentation.set_emote_input_mode(json_ui::InputMode::Mouse);
        }
        if let Some(slot) = slot_key(event.key_code) {
            input.runtime.emotes_mut().activate_slot(slot, now);
        } else if let Some(action) = wheel_key(event.key_code) {
            input.runtime.emotes_mut().handle_action(action, now);
        }
    }
    if input.runtime.emotes().is_open() {
        let hit = if let Some(presentation) = input.presentation.as_deref_mut() {
            presentation.set_emote_pointer(pointer);
            pointer.and_then(|point| presentation.hit_test_emote(point))
        } else {
            None
        };
        // A stationary pointer must not undo keyboard/controller navigation.
        if (pointer != input.observed.pointer && input.motion.delta != Vec2::ZERO)
            || input.mouse.just_pressed(MouseButton::Left)
        {
            if (input.motion.delta != Vec2::ZERO || input.mouse.just_pressed(MouseButton::Left))
                && let Some(presentation) = input.presentation.as_deref_mut()
            {
                presentation.set_emote_input_mode(json_ui::InputMode::Mouse);
            }
            input.runtime.emotes_mut().hover_slot(match hit {
                Some(EmoteHit::Slot(slot)) => Some(slot),
                _ => None,
            });
        }
        input.observed.pointer = pointer;
        if input.mouse.just_pressed(MouseButton::Left) {
            match hit {
                Some(EmoteHit::Slot(slot)) => {
                    input.runtime.emotes_mut().activate_slot(slot, now);
                }
                Some(EmoteHit::ChangeEmotes) => input.runtime.emotes_mut().change_emotes(),
                Some(EmoteHit::Close) => {
                    input
                        .runtime
                        .emotes_mut()
                        .handle_action(UiAction::Cancel, now);
                }
                None => {}
            }
        }
        for pad in input.pads.iter().filter(|_| !toggled) {
            for (button, action) in [
                (GamepadButton::South, UiAction::Accept),
                (GamepadButton::East, UiAction::Cancel),
                (GamepadButton::DPadUp, UiAction::Navigate([0, -1])),
                (GamepadButton::DPadRight, UiAction::Navigate([1, 0])),
                (GamepadButton::DPadDown, UiAction::Navigate([0, 1])),
                (GamepadButton::DPadLeft, UiAction::Navigate([-1, 0])),
            ] {
                let button = input.menu.as_deref().map_or(button, |menu| {
                    gamepad_button(&menu.settings_snapshot().0, button)
                });
                if pad.just_pressed(button) {
                    if let Some(presentation) = input.presentation.as_deref_mut() {
                        presentation.set_emote_input_mode(json_ui::InputMode::Gamepad);
                    }
                    input.runtime.emotes_mut().handle_action(action, now);
                }
            }
        }
    } else if let Some(presentation) = input.presentation.as_deref_mut() {
        presentation.set_emote_pointer(None);
        input.observed.pointer = None;
    }
    if let Some(preferences) = input.runtime.emotes_mut().take_preferences()
        && let Some(menu) = input.menu.as_deref_mut()
    {
        menu.set_emote_slot_preferences(preferences.clone());
        input.observed.preferences = Some(Some(preferences));
    }
    if owned {
        input.consumed.0 = true;
        input.keys.reset_all();
        input.mouse.reset_all();
        input.motion.delta = Vec2::ZERO;
        input.window.1.grab_mode = if input.runtime.emotes().is_open() {
            CursorGrabMode::None
        } else {
            CursorGrabMode::Locked
        };
        input.window.1.visible = input.runtime.emotes().is_open();
    }
}

/// The current gameplay snapshot is available after UI authority has finalized.
pub(crate) fn cancel_emote_from_gameplay(
    semantic: Res<SemanticInputSnapshot>,
    mut runtime: ResMut<UiRuntime>,
) {
    if should_stop_emote(&semantic) {
        runtime.emotes_mut().stop();
    }
}

fn toggle(runtime: &mut UiRuntime) {
    if runtime.emotes().is_open() {
        runtime.emotes_mut().close();
    } else {
        runtime.emotes_mut().open();
    }
}

fn should_stop_emote(input: &SemanticInputSnapshot) -> bool {
    input.raw_movement().iter().any(|axis| *axis != 0.0)
        || [Action::Jump, Action::Attack, Action::Use, Action::Sneak]
            .iter()
            .any(|action| input.phase(*action).held)
}

fn slot_key(key: KeyCode) -> Option<usize> {
    match key {
        KeyCode::Digit1 | KeyCode::Numpad1 => Some(0),
        KeyCode::Digit2 | KeyCode::Numpad2 => Some(1),
        KeyCode::Digit3 | KeyCode::Numpad3 => Some(2),
        KeyCode::Digit4 | KeyCode::Numpad4 => Some(3),
        _ => None,
    }
}

fn wheel_key(key: KeyCode) -> Option<UiAction> {
    match key {
        KeyCode::Escape => Some(UiAction::Cancel),
        KeyCode::Enter | KeyCode::NumpadEnter | KeyCode::Space => Some(UiAction::Accept),
        KeyCode::ArrowUp => Some(UiAction::Navigate([0, -1])),
        KeyCode::ArrowRight => Some(UiAction::Navigate([1, 0])),
        KeyCode::ArrowDown => Some(UiAction::Navigate([0, 1])),
        KeyCode::ArrowLeft => Some(UiAction::Navigate([-1, 0])),
        _ => None,
    }
}

#[cfg(test)]
mod tests;
