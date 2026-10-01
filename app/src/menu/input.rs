use bevy::{
    ecs::message::{MessageCursor, Messages},
    input::{
        ButtonState,
        gamepad::{Gamepad, GamepadButton},
        keyboard::KeyboardInput,
        mouse::{MouseScrollUnit, MouseWheel},
        touch::Touches,
    },
    prelude::{
        ButtonInput, KeyCode, Local, MessageReader, MouseButton, Query, Res, ResMut, Resource,
        Single, With,
    },
    window::{CursorGrabMode, CursorOptions, PrimaryWindow, Window},
};
use ui::{ChatClipboard, UiPoint};

use super::{MAX_SERVER_ADDRESS_BYTES, MAX_SERVER_NAME_BYTES, MenuField, MenuRuntime};
use crate::local_worlds::{MAX_SEED_CHARS, MAX_WORLD_NAME_CHARS};
use crate::ui_runtime::{PlatformClipboard, presentation::UiPresentationRuntime};

#[derive(Resource)]
pub(crate) struct MenuClipboard(
    Box<dyn FnMut(usize) -> Option<String> + Send + Sync + 'static>,
    Box<dyn FnMut(String) + Send + Sync + 'static>,
);

impl MenuClipboard {
    pub(crate) fn with_access(
        reader: impl FnMut(usize) -> Option<String> + Send + Sync + 'static,
        writer: impl FnMut(String) + Send + Sync + 'static,
    ) -> Self {
        Self(Box::new(reader), Box::new(writer))
    }

    fn read_text_bounded(&mut self, maximum_bytes: usize) -> Option<String> {
        (self.0)(maximum_bytes)
    }

    fn write_text(&mut self, text: String) {
        (self.1)(text);
    }
}

impl Default for MenuClipboard {
    fn default() -> Self {
        let mut reader = PlatformClipboard;
        let mut writer = PlatformClipboard;
        Self::with_access(
            move |maximum_bytes| {
                reader
                    .read_text_bounded(maximum_bytes)
                    .ok()
                    .flatten()
                    .map(|text| text.to_string())
            },
            move |text| {
                let _ = writer.write_text(text);
            },
        )
    }
}

#[derive(Default)]
pub(crate) struct MenuModifiers(u8);

impl MenuModifiers {
    const CONTROL_LEFT: u8 = 1 << 0;
    const CONTROL_RIGHT: u8 = 1 << 1;
    const SUPER_LEFT: u8 = 1 << 2;
    const SUPER_RIGHT: u8 = 1 << 3;
    const ALT_LEFT: u8 = 1 << 4;
    const ALT_RIGHT: u8 = 1 << 5;
    const SHIFT_LEFT: u8 = 1 << 6;
    const SHIFT_RIGHT: u8 = 1 << 7;

    fn capture_pressed(&mut self, keys: &ButtonInput<KeyCode>) {
        for key in [
            KeyCode::ControlLeft,
            KeyCode::ControlRight,
            KeyCode::SuperLeft,
            KeyCode::SuperRight,
            KeyCode::AltLeft,
            KeyCode::AltRight,
            KeyCode::ShiftLeft,
            KeyCode::ShiftRight,
        ] {
            if keys.pressed(key) {
                self.0 |= Self::mask(key);
            }
        }
    }

    fn observe(&mut self, input: &KeyboardInput) {
        let mask = Self::mask(input.key_code);
        if input.state == ButtonState::Pressed {
            self.0 |= mask;
        } else {
            self.0 &= !mask;
        }
    }

    fn shortcut(&self) -> bool {
        self.0 & 0b0000_1111 != 0 && self.0 & 0b0011_0000 == 0
    }

    fn shift(&self) -> bool {
        self.0 & 0b1100_0000 != 0
    }

    const fn mask(key: KeyCode) -> u8 {
        match key {
            KeyCode::ControlLeft => Self::CONTROL_LEFT,
            KeyCode::ControlRight => Self::CONTROL_RIGHT,
            KeyCode::SuperLeft => Self::SUPER_LEFT,
            KeyCode::SuperRight => Self::SUPER_RIGHT,
            KeyCode::AltLeft => Self::ALT_LEFT,
            KeyCode::AltRight => Self::ALT_RIGHT,
            KeyCode::ShiftLeft => Self::SHIFT_LEFT,
            KeyCode::ShiftRight => Self::SHIFT_RIGHT,
            _ => 0,
        }
    }
}

impl MenuRuntime {
    pub(super) fn focus_field(&mut self, field: MenuField) {
        self.field = Some(field);
        self.text_selected = false;
    }

    fn has_focused_field(&self) -> bool {
        self.field.is_some()
    }

    fn selected_text_target(&self) -> Option<&str> {
        match self.field? {
            MenuField::Name => Some(&self.name),
            MenuField::Address => Some(&self.address),
            MenuField::WorldName => Some(&self.local_ui.name),
            MenuField::WorldSeed => Some(&self.local_ui.seed),
        }
    }

    fn text_target(&mut self, field: MenuField) -> &mut String {
        match field {
            MenuField::Name => &mut self.name,
            MenuField::Address => &mut self.address,
            MenuField::WorldName => &mut self.local_ui.name,
            MenuField::WorldSeed => &mut self.local_ui.seed,
        }
    }

    fn select_all_text(&mut self) {
        self.text_selected = self
            .selected_text_target()
            .is_some_and(|text| !text.is_empty());
    }

    fn selected_text(&self) -> Option<&str> {
        self.text_selected
            .then(|| self.selected_text_target())
            .flatten()
    }

    fn remaining_text_capacity(&self) -> usize {
        let Some(field) = self.field else {
            return 0;
        };
        let maximum = max_bytes(field);
        if self.text_selected {
            maximum
        } else {
            maximum.saturating_sub(self.selected_text_target().map_or(0, str::len))
        }
    }

    fn edit_text(&mut self, text: &str) {
        let Some(field) = self.field else {
            return;
        };
        let maximum = max_bytes(field);
        let selected = self.text_selected;
        let target = self.text_target(field);
        let mut insertion = String::new();
        let base_length = if selected { 0 } else { target.len() };
        for character in text.chars().filter(|character| !character.is_control()) {
            if base_length
                .saturating_add(insertion.len())
                .saturating_add(character.len_utf8())
                > maximum
            {
                break;
            }
            insertion.push(character);
        }
        if insertion.is_empty() {
            return;
        }
        if selected {
            target.clear();
        }
        target.push_str(&insertion);
        self.text_selected = false;
    }

    fn backspace_text(&mut self) {
        let Some(field) = self.field else {
            return;
        };
        let selected = self.text_selected;
        let target = self.text_target(field);
        if selected {
            target.clear();
        } else {
            let _ = target.pop();
        }
        self.text_selected = false;
    }
}

/// A field's byte budget; world fields allow their vanilla character limits in any script.
fn max_bytes(field: MenuField) -> usize {
    match field {
        MenuField::Name => MAX_SERVER_NAME_BYTES,
        MenuField::Address => MAX_SERVER_ADDRESS_BYTES,
        MenuField::WorldName => MAX_WORLD_NAME_CHARS * 4,
        MenuField::WorldSeed => MAX_SEED_CHARS,
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn drive_menu_input(
    mut keyboard_messages: MessageReader<KeyboardInput>,
    wheel_messages: Option<Res<Messages<MouseWheel>>>,
    mut wheel_cursor: Local<MessageCursor<MouseWheel>>,
    window: Single<(&Window, &mut CursorOptions), With<PrimaryWindow>>,
    mut keys: ResMut<ButtonInput<KeyCode>>,
    mut mouse_buttons: ResMut<ButtonInput<MouseButton>>,
    touches: Res<Touches>,
    gamepads: Query<&Gamepad>,
    mut presentation: ResMut<UiPresentationRuntime>,
    mut clipboard: ResMut<MenuClipboard>,
    mut menu: ResMut<MenuRuntime>,
    runtime: Option<Res<crate::ui_runtime::UiRuntime>>,
    mut modifiers: Local<MenuModifiers>,
) {
    let (window, mut cursor) = window.into_inner();
    let wheel: Vec<(f32, bool)> = wheel_messages
        .as_deref()
        .map(|messages| {
            wheel_cursor
                .read(messages)
                .map(|wheel| (wheel.y, wheel.unit == MouseScrollUnit::Pixel))
                .collect()
        })
        .unwrap_or_default();
    if runtime.as_ref().is_some_and(|runtime| {
        runtime.server_forms().owns_input()
            && (!menu.is_visible() || runtime.server_forms().settings_form_active())
    }) {
        keyboard_messages.clear();
        return;
    }
    menu.pressed = None;
    if !window.focused {
        *modifiers = MenuModifiers::default();
        keyboard_messages.clear();
        menu.pointer_down = false;
        return;
    }
    // Zero health in play opens the death screen; recovery closes it.
    if let Some(health) = runtime.as_ref().and_then(|runtime| runtime.hud().health()) {
        if health.current() == 0 {
            menu.open_death();
        } else {
            menu.note_player_alive();
        }
    }
    if !menu.is_visible() {
        // Gameplay/chat handled these messages already. In particular, do not
        // replay the Escape that opens pause as "back" on the following frame.
        *modifiers = MenuModifiers::default();
        keyboard_messages.clear();
        menu.hovered = None;
        menu.pointer_down = false;
        if keys.just_pressed(KeyCode::Escape) {
            modifiers.capture_pressed(&keys);
            menu.open_pause();
            cursor.grab_mode = CursorGrabMode::None;
            cursor.visible = true;
            keys.reset_all();
        }
        return;
    }

    modifiers.capture_pressed(&keys);
    cursor.grab_mode = CursorGrabMode::None;
    cursor.visible = true;
    let pointer = window
        .cursor_position()
        .and_then(|position| UiPoint::new(position.x, position.y).ok());
    menu.hovered = pointer.and_then(|position| presentation.hit_test_menu(position));
    let pointer_pressed = mouse_buttons.pressed(MouseButton::Left);
    let pointer_just_pressed =
        mouse_buttons.just_pressed(MouseButton::Left) || (pointer_pressed && !menu.pointer_down);
    menu.pointer_down = pointer_pressed;
    if let Some(point) = pointer {
        for (notches, pixels) in wheel {
            presentation.scroll_menu(point, notches, pixels);
        }
    }
    // A scrollbar press or drag scrolls instead of pressing what lies beneath.
    let on_scrollbar = presentation.drag_menu_scroll(pointer, pointer_pressed)
        || (pointer_just_pressed
            && pointer.is_some_and(|point| presentation.press_menu_scrollbar(point)));
    if on_scrollbar {
        menu.hovered = None;
    }
    let press = |menu: &mut MenuRuntime, action| {
        if let Some(sound) = presentation.menu_sound(action) {
            crate::audio::ui_sound(sound);
        }
        menu.activate(action);
    };
    if pointer_just_pressed
        && !on_scrollbar
        && let Some(action) = menu.hovered
    {
        press(&mut menu, action);
    }
    for touch in touches.iter_just_pressed() {
        let position = touch.position();
        if let Ok(position) = UiPoint::new(position.x, position.y)
            && let Some(action) = presentation.hit_test_menu(position)
        {
            press(&mut menu, action);
        }
    }
    for gamepad in &gamepads {
        if gamepad.just_pressed(GamepadButton::DPadUp)
            || gamepad.just_pressed(GamepadButton::DPadLeft)
        {
            menu.move_focus(-1);
        }
        if gamepad.just_pressed(GamepadButton::DPadDown)
            || gamepad.just_pressed(GamepadButton::DPadRight)
        {
            menu.move_focus(1);
        }
        if gamepad.just_pressed(GamepadButton::South) {
            menu.activate_focused();
        }
        if gamepad.just_pressed(GamepadButton::East) {
            menu.go_back_from_input();
        }
    }
    for input in keyboard_messages.read() {
        modifiers.observe(input);
        if input.state != ButtonState::Pressed {
            continue;
        }
        if modifiers.shortcut() && menu.has_focused_field() {
            match input.key_code {
                KeyCode::KeyA => {
                    menu.select_all_text();
                    continue;
                }
                KeyCode::KeyC => {
                    if let Some(text) = menu.selected_text() {
                        clipboard.write_text(text.to_owned());
                    }
                    continue;
                }
                KeyCode::KeyV => {
                    let maximum = menu.remaining_text_capacity();
                    if let Some(text) = clipboard.read_text_bounded(maximum) {
                        menu.edit_text(&text);
                    }
                    continue;
                }
                _ => {}
            }
            if input.text.is_some() {
                continue;
            }
        }
        match input.key_code {
            KeyCode::Escape => menu.go_back_from_input(),
            KeyCode::ArrowUp | KeyCode::ArrowLeft => menu.move_focus(-1),
            KeyCode::ArrowDown | KeyCode::ArrowRight => menu.move_focus(1),
            KeyCode::Tab => menu.move_focus(if modifiers.shift() { -1 } else { 1 }),
            KeyCode::Enter | KeyCode::NumpadEnter => menu.activate_focused(),
            KeyCode::Backspace if menu.field.is_some() => menu.backspace_text(),
            _ if menu.has_focused_field() && !modifiers.shortcut() => {
                if let Some(text) = input.text.as_deref() {
                    menu.edit_text(text);
                }
            }
            _ => {}
        }
    }
    if let Some(scale) = menu.take_gui_scale_change() {
        presentation.set_gui_scale_preference(Some(scale));
    }
    // The menu owns the pointer and keyboard for this frame. This also keeps
    // the camera's recapture-on-click path from turning a menu click into a
    // gameplay attack or mouse grab.
    keys.reset_all();
    mouse_buttons.reset_all();
}

impl MenuRuntime {
    fn go_back_from_input(&mut self) {
        self.go_back();
    }
}
