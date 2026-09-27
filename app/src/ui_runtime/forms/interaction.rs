use super::LocalFormAction;
use crate::{
    menu::MenuRuntime,
    ui_runtime::{UiRuntime, presentation::UiPresentationRuntime},
};
use bevy::{
    input::mouse::{AccumulatedMouseMotion, MouseScrollUnit, MouseWheel},
    prelude::{
        ButtonInput, KeyCode, Local, MessageReader, MouseButton, Res, ResMut, Single, Window, With,
    },
    window::{CursorOptions, PrimaryWindow},
};

#[allow(clippy::too_many_arguments)]
pub(crate) fn drive_server_form_input(
    window: Single<(&Window, &mut CursorOptions), With<PrimaryWindow>>,
    mut keys: ResMut<ButtonInput<KeyCode>>,
    mut mouse: ResMut<ButtonInput<MouseButton>>,
    mut motion: ResMut<AccumulatedMouseMotion>,
    mut wheel: MessageReader<MouseWheel>,
    menu: Option<Res<MenuRuntime>>,
    presentation: Res<UiPresentationRuntime>,
    mut runtime: ResMut<UiRuntime>,
    mut owned_last_frame: Local<bool>,
) {
    let (window, mut cursor) = window.into_inner();
    if menu.as_ref().is_some_and(|menu| menu.is_visible()) {
        runtime.server_forms_mut().reject_active_busy();
        wheel.clear();
        *owned_last_frame = false;
        return;
    }
    if !runtime.server_forms().owns_input() {
        wheel.clear();
        if *owned_last_frame && !runtime.ui_focused() && window.focused {
            crate::ui_runtime::interaction::restore_gameplay_input_after_chat(
                &mut cursor,
                &mut keys,
                &mut mouse,
                &mut motion,
            );
        }
        *owned_last_frame = false;
        return;
    }
    *owned_last_frame = true;
    if window.focused
        && let Some(entry) = runtime.server_forms().active()
    {
        let identity = entry.identity;
        if keys.just_pressed(KeyCode::ArrowUp)
            || (keys.just_pressed(KeyCode::Tab) && keys.pressed(KeyCode::ShiftLeft))
        {
            runtime.server_forms_mut().move_focus(-1);
        } else if keys.just_pressed(KeyCode::ArrowDown) || keys.just_pressed(KeyCode::Tab) {
            runtime.server_forms_mut().move_focus(1);
        }
        if (keys.just_pressed(KeyCode::ArrowUp)
            || keys.just_pressed(KeyCode::ArrowDown)
            || keys.just_pressed(KeyCode::Tab))
            && let Some(offset) =
                presentation.form_focus_scroll(identity, runtime.server_forms().focus())
        {
            runtime.server_forms_mut().set_scroll(offset);
        }
        for event in wheel.read() {
            let pixels = match event.unit {
                MouseScrollUnit::Line => event.y * 44.0,
                MouseScrollUnit::Pixel => event.y,
            };
            runtime
                .server_forms_mut()
                .scroll_rows((-pixels).clamp(-4096.0, 4096.0) as i32);
        }
        let action = if keys.just_pressed(KeyCode::Escape) {
            Some((identity, LocalFormAction::Dismiss))
        } else if keys.just_pressed(KeyCode::Enter)
            || keys.just_pressed(KeyCode::NumpadEnter)
            || keys.just_pressed(KeyCode::Space)
        {
            presentation.form_button_count(identity).and_then(|count| {
                let index = runtime.server_forms().focus();
                if index < count && !presentation.form_button_visible(identity, index) {
                    if let Some(offset) = presentation.form_focus_scroll(identity, index) {
                        runtime.server_forms_mut().set_scroll(offset);
                    }
                    return None;
                }
                Some((
                    identity,
                    if index < count {
                        LocalFormAction::SubmitButton(index as u32)
                    } else {
                        LocalFormAction::Dismiss
                    },
                ))
            })
        } else if mouse.just_pressed(MouseButton::Left) {
            window
                .cursor_position()
                .and_then(|point| ui::UiPoint::new(point.x, point.y).ok())
                .and_then(|point| presentation.hit_test_form(point))
        } else {
            None
        };
        if let Some((identity, action)) = action {
            let _ = runtime.respond_to_server_form(identity, action);
        }
    } else {
        wheel.clear();
    }
    // Also retain ownership through the answer frame; pending enqueue owns
    // input until the later network phase accepts it or retires the session.
    crate::ui_runtime::interaction::suppress_gameplay_input_for_chat(
        &runtime,
        &mut cursor,
        &mut keys,
        &mut mouse,
        &mut motion,
    );
}
