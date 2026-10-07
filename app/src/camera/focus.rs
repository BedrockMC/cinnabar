//! Desktop focus arbitration before input sampling and before OS cursor updates.
use bevy::{
    input::{
        InputSystems,
        mouse::{AccumulatedMouseMotion, MouseButtonInput},
    },
    prelude::*,
    window::{CursorGrabMode, CursorOptions, PrimaryWindow, WindowFocused, WindowOccluded},
};
use client_presentation::camera::CursorFocus;

use super::{AutoFly, DrivenInput};

/// Installs focus tracking without opening or calling into a native window.
pub(super) fn install(app: &mut App) {
    app.init_resource::<CursorFocus>()
        .add_message::<WindowFocused>()
        .add_message::<WindowOccluded>()
        .add_message::<MouseButtonInput>()
        .add_systems(PreUpdate, track_focus.after(InputSystems))
        .add_systems(PostUpdate, enforce_cursor_ownership);
}

/// Retains primary-window loss events before gameplay or screen input is sampled.
#[allow(clippy::too_many_arguments)]
fn track_focus(
    mut focus: ResMut<CursorFocus>,
    mut focused: MessageReader<WindowFocused>,
    mut occluded: MessageReader<WindowOccluded>,
    mut pointer_edges: MessageReader<MouseButtonInput>,
    window: Single<(Entity, &Window, &mut CursorOptions), With<PrimaryWindow>>,
    driven: Option<Res<DrivenInput>>,
    mut keys: ResMut<ButtonInput<KeyCode>>,
    mut buttons: ResMut<ButtonInput<MouseButton>>,
    mut motion: ResMut<AccumulatedMouseMotion>,
    gamepads: Query<&Gamepad>,
    touches: Option<Res<Touches>>,
    mut was_available: Local<bool>,
    #[cfg(windows)] _window_thread: bevy::ecs::system::NonSendMarker,
) {
    let (entity, window, mut cursor) = window.into_inner();
    focus.begin_frame(window.focused);
    for event in focused.read().filter(|event| event.window == entity) {
        focus.focus_changed(event.focused);
    }
    for event in occluded.read().filter(|event| event.window == entity) {
        focus.occlusion_changed(event.occluded);
    }
    let mut pointer_activated = false;
    for event in pointer_edges.read() {
        pointer_activated |= event.window == entity;
    }
    focus.record_activation(
        keys.get_just_pressed().next().is_some()
            || pointer_activated
            || buttons.get_just_pressed().next().is_some()
            || buttons.get_just_released().next().is_some()
            || touches
                .is_some_and(|touches| touches.any_just_pressed() || touches.any_just_released())
            || gamepads
                .iter()
                .any(|pad| pad.get_just_pressed().next().is_some()),
    );
    let available = focus.available();
    let input_lost = std::mem::replace(&mut *was_available, available) && !available;
    if driven.is_some() {
        return;
    }
    if !available {
        if native_release_needed(&cursor, &buttons, input_lost) {
            release_native_capture();
        }
        super::release_cursor(&mut cursor);
        keys.reset_all();
        buttons.reset_all();
        motion.delta = Vec2::ZERO;
    }
}

/// Includes controller ownership without requesting a physical OS cursor grab.
pub(crate) fn mouse_input_active(
    window: &Window,
    cursor: &CursorOptions,
    focus: Option<&CursorFocus>,
    driven: bool,
) -> bool {
    driven
        || (super::input_is_active(window, cursor)
            && focus.is_none_or(CursorFocus::capture_allowed))
}

/// Samples UI cursor authority immediately before presentation updates capture.
#[allow(clippy::too_many_arguments)]
pub(crate) fn update_cursor_capture(
    player_runtime: bevy::prelude::Res<crate::player_runtime::PlayerRuntime>,
    window: Single<(&Window, &mut CursorOptions), With<PrimaryWindow>>,
    keys: ResMut<ButtonInput<KeyCode>>,
    mouse_buttons: ResMut<ButtonInput<MouseButton>>,
    mouse_motion: ResMut<AccumulatedMouseMotion>,
    auto_fly: ResMut<AutoFly>,
    ui: Option<Res<client_ui::ui_runtime::UiRuntime>>,
    menu: Option<Res<crate::menu::MenuRuntime>>,
    presentation: Option<Res<client_ui::ui_runtime::presentation::UiPresentationRuntime>>,
    consent: Option<Res<crate::server_experiences::input::ConsentInput>>,
    driven: Option<Res<DrivenInput>>,
    focus: Option<ResMut<client_presentation::camera::CursorFocus>>,
) {
    let mut policy = client_presentation::observations::CursorPolicy {
        capture_allowed: true,
        driven: driven.is_some(),
        consent: consent.is_some_and(|consent| consent.0),
        absorbs_input: crate::screen_policy::absorbs_input(
            &player_runtime,
            ui.as_deref(),
            menu.as_deref(),
            presentation.as_deref(),
        ),
        steals_mouse: ui.as_deref().map(|ui| {
            ui.steals_mouse(
                &player_runtime,
                menu.as_deref().map(|menu| {
                    menu as &dyn client_ui::ui_runtime::presentation::forms::scene_policy::MenuScene
                }),
            )
        }),
    };
    if let Some(mut focus) = focus {
        policy.capture_allowed = focus.allow_capture(
            policy.absorbs_input,
            mouse_buttons.just_pressed(MouseButton::Left),
        );
    }
    client_presentation::camera::update_cursor_capture(
        policy,
        window,
        keys,
        mouse_buttons,
        mouse_motion,
        auto_fly,
    );
}

/// Prevents any screen or developer adapter from submitting an unauthorized OS grab.
fn enforce_cursor_ownership(
    focus: Res<CursorFocus>,
    driven: Option<Res<DrivenInput>>,
    mut cursors: Query<&mut CursorOptions, With<PrimaryWindow>>,
) {
    if driven.is_some() || !focus.capture_allowed() {
        for mut cursor in &mut cursors {
            super::release_cursor(&mut cursor);
        }
    }
}

/// A focus transition releases native ownership even after UI consumed local input.
fn native_release_needed(
    cursor: &CursorOptions,
    buttons: &ButtonInput<MouseButton>,
    input_lost: bool,
) -> bool {
    input_lost || cursor.grab_mode != CursorGrabMode::None || buttons.get_pressed().next().is_some()
}

/// Windows retains clipping and button capture independently of keyboard focus.
fn release_native_capture() {
    #[cfg(all(windows, not(test)))]
    {
        use windows_sys::Win32::UI::{
            Input::KeyboardAndMouse::{GetCapture, ReleaseCapture},
            WindowsAndMessaging::ClipCursor,
        };
        // A null rectangle releases the clip without moving the pointer.
        if unsafe { ClipCursor(std::ptr::null()) } == 0 {
            warn!(
                "could not release cursor clip: {}",
                std::io::Error::last_os_error()
            );
        }
        // Capture belongs to the window thread, enforced by the focus system.
        if !unsafe { GetCapture() }.is_null() && unsafe { ReleaseCapture() } == 0 {
            warn!(
                "could not release mouse capture: {}",
                std::io::Error::last_os_error()
            );
        }
    }
}

#[cfg(test)]
mod tests;
