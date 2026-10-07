//! Desktop focus arbitration before input sampling and before OS cursor updates.
use bevy::{
    input::{InputSystems, mouse::AccumulatedMouseMotion},
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
        .add_systems(PreUpdate, track_focus.after(InputSystems))
        .add_systems(PostUpdate, enforce_cursor_ownership);
}

/// Retains primary-window loss events before gameplay or screen input is sampled.
#[allow(clippy::too_many_arguments)]
fn track_focus(
    mut focus: ResMut<CursorFocus>,
    mut focused: MessageReader<WindowFocused>,
    mut occluded: MessageReader<WindowOccluded>,
    window: Single<(Entity, &Window, &mut CursorOptions), With<PrimaryWindow>>,
    driven: Option<Res<DrivenInput>>,
    mut keys: ResMut<ButtonInput<KeyCode>>,
    mut buttons: ResMut<ButtonInput<MouseButton>>,
    mut motion: ResMut<AccumulatedMouseMotion>,
    gamepads: Query<&Gamepad>,
) {
    let (entity, window, mut cursor) = window.into_inner();
    focus.begin_frame(window.focused);
    for event in focused.read().filter(|event| event.window == entity) {
        focus.focus_changed(event.focused);
    }
    for event in occluded.read().filter(|event| event.window == entity) {
        focus.occlusion_changed(event.occluded);
    }
    focus.record_activation(
        keys.get_just_pressed().next().is_some()
            || buttons.just_pressed(MouseButton::Left)
            || gamepads
                .iter()
                .any(|pad| pad.get_just_pressed().next().is_some()),
    );
    if driven.is_some() {
        return;
    }
    if !focus.available() {
        if cursor.grab_mode != CursorGrabMode::None {
            release_native_clip();
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

/// Windows may retain a global clip after input moves to an injected overlay.
fn release_native_clip() {
    #[cfg(all(windows, not(test)))]
    // No rectangle releases the clip; this call does not move or inject the pointer.
    if unsafe { windows_sys::Win32::UI::WindowsAndMessaging::ClipCursor(std::ptr::null()) } == 0 {
        warn!(
            "could not release cursor clip: {}",
            std::io::Error::last_os_error()
        );
    }
}

#[cfg(test)]
mod tests;
