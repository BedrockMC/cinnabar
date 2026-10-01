//! Input for a form drawn by the JSON-UI engine. Raw pointer and keyboard
//! edges become vanilla input buttons (`button.menu_select`, `button.menu_ok`,
//! …) that the engine's dispatcher routes through the template's button
//! mappings to its components; this module is the form's screen controller,
//! turning the resulting screen events into form values and answers.

use bevy::input::{ButtonInput, keyboard::KeyCode, mouse::MouseScrollUnit};
use json_ui::{
    ButtonEvent, ButtonInput as EngineButton, Dispatch, HitKind, HitRegion, InputMode,
    PointerInput, ScreenEvent, hit_test, scroll_target,
};
use protocol::{CustomFormElement, MenuElement, ServerFormModel};
use ui::{ChatClipboard, UiPoint};

use super::engine_focus;
use super::values::{EngineFrame, FormDrag, slider_value_at};
use super::{FormValue, LocalFormAction};
use crate::ui_runtime::{PlatformClipboard, UiRuntime};

/// Longest paste accepted into an edit box, before its own `max_length`.
const MAX_PASTE_BYTES: usize = 4096;
/// The input button a primary pointer press is.
const SELECT: &str = "button.menu_select";

/// The primary pointer button's edges this frame and whether it is down.
#[derive(Clone, Copy, Debug, Default)]
pub(super) struct PointerButtons {
    pub(super) pressed: bool,
    pub(super) released: bool,
    pub(super) held: bool,
}

/// One frame of raw input for the engine path.
pub(super) struct EngineInput<'a> {
    pub(super) cursor: Option<UiPoint>,
    pub(super) keys: &'a ButtonInput<KeyCode>,
    pub(super) pointer: PointerButtons,
    pub(super) wheel: Vec<(f32, MouseScrollUnit)>,
    /// Pressed keys this frame with their produced text.
    pub(super) typed: Vec<(KeyCode, Option<String>)>,
    /// Seconds on the app clock.
    pub(super) now: f64,
}

pub(super) fn drive(runtime: &mut UiRuntime, frame: &EngineFrame, input: EngineInput<'_>) {
    let Some(entry) = runtime.server_forms().active() else {
        return;
    };
    let identity = entry.identity;
    let model = entry.model.clone();
    let point = input.cursor.map(|cursor| frame.to_virtual(cursor));
    let control = input.keys.pressed(KeyCode::ControlLeft)
        || input.keys.pressed(KeyCode::ControlRight)
        || input.keys.pressed(KeyCode::SuperLeft)
        || input.keys.pressed(KeyCode::SuperRight);
    let mut events = Vec::new();
    {
        let engine = runtime.server_forms_mut().engine_mut();
        let delta = (input.now - engine.clock).max(0.0);
        engine.clock = input.now;
        engine.dispatcher.tick(&frame.hits, &mut engine.view, delta);
        let pointer = PointerInput {
            point,
            held: input.pointer.held,
            mode: InputMode::Mouse,
            now: input.now,
        };
        events.extend(
            engine
                .dispatcher
                .pointer(&frame.hits, &mut engine.view, pointer)
                .events,
        );
        // Keyboard focus shows as hover while the pointer rests on nothing.
        if engine.view.hovered.is_none() {
            engine.view.hovered = engine.view.focused.clone();
        }
    }
    if let Some(point) = point {
        drag(runtime, frame, point, input.pointer.held);
    }
    // Each release answers only for the control its press went down on.
    let mut release = None;
    if input.pointer.pressed
        && let Some(point) = point
    {
        press_scroll(runtime, frame, point);
        events.extend(button(runtime, frame, SELECT, true, Some(point), input.now).events);
    }
    if input.pointer.released {
        runtime.server_forms_mut().engine_mut().drag = None;
        let pressed = runtime.server_forms().engine().view.pressed.clone();
        let up = button(runtime, frame, SELECT, false, point, input.now).events;
        release = Some((events.len()..events.len() + up.len(), pressed));
        events.extend(up);
    }
    for (notches, unit) in &input.wheel {
        if let Some(point) = point
            && let Some(view) = scroll_target(&frame.hits, point)
        {
            let delta = match unit {
                MouseScrollUnit::Line => {
                    f64::from(-notches)
                        * frame
                            .report
                            .scrolls
                            .get(&view.key)
                            .map_or(15.0, |m| m.speed)
                }
                MouseScrollUnit::Pixel => f64::from(-notches / frame.scale),
            };
            scroll_by(runtime, frame, &view.key, delta);
        }
    }
    for (key, text) in input.typed {
        events.extend(keyboard(
            runtime,
            frame,
            key,
            text.as_deref(),
            control,
            input.now,
        ));
    }
    let mut action = None;
    for (at, event) in events.iter().enumerate() {
        let pressed = release
            .as_ref()
            .filter(|(range, _)| range.contains(&at))
            .and_then(|(_, pressed)| pressed.as_deref());
        if let Some(found) = controller(runtime, frame, &model, event, pressed) {
            action = Some(found);
        }
    }
    if let Some(action) = action {
        let _ = runtime.respond_to_server_form(identity, action);
    }
}

/// One input button edge through the dispatcher.
fn button(
    runtime: &mut UiRuntime,
    frame: &EngineFrame,
    id: &str,
    down: bool,
    point: Option<[f64; 2]>,
    now: f64,
) -> Dispatch {
    let engine = runtime.server_forms_mut().engine_mut();
    let input = EngineButton {
        id,
        down,
        point,
        mode: InputMode::Mouse,
        now,
    };
    engine
        .dispatcher
        .button(&frame.hits, &mut engine.view, input)
}

/// A key as the input buttons vanilla's keyboard mapping raises, or typed text
/// for the selected edit box.
fn keyboard(
    runtime: &mut UiRuntime,
    frame: &EngineFrame,
    key: KeyCode,
    text: Option<&str>,
    control: bool,
    now: f64,
) -> Vec<ScreenEvent> {
    let editing = runtime
        .server_forms()
        .engine()
        .view
        .components
        .selected()
        .is_some_and(|key| edit_region(frame, key).is_some());
    let typed = match key {
        KeyCode::Backspace if editing => Some("\u{8}".to_owned()),
        KeyCode::Enter | KeyCode::NumpadEnter if editing => Some("\r".to_owned()),
        KeyCode::KeyV if control && editing => PlatformClipboard
            .read_text_bounded(MAX_PASTE_BYTES)
            .ok()
            .flatten()
            .map(|text| text.to_string()),
        _ if editing && !control => text
            .filter(|text| !text.chars().any(char::is_control))
            .map(str::to_owned),
        _ => None,
    };
    if let Some(typed) = typed {
        let engine = runtime.server_forms_mut().engine_mut();
        return engine
            .dispatcher
            .text(&frame.hits, &mut engine.view, &typed, None)
            .events;
    }
    let id = match key {
        KeyCode::Escape => "button.menu_cancel",
        KeyCode::Enter | KeyCode::NumpadEnter | KeyCode::Space => "button.menu_ok",
        KeyCode::ArrowUp => "button.menu_up",
        KeyCode::ArrowDown => "button.menu_down",
        KeyCode::ArrowLeft => "button.menu_left",
        KeyCode::ArrowRight => "button.menu_right",
        KeyCode::Tab => {
            engine_focus::tab(runtime, frame);
            return Vec::new();
        }
        _ => return Vec::new(),
    };
    let down = button(runtime, frame, id, true, None, now);
    let consumed = down.consumed;
    let mut events = down.events;
    events.extend(button(runtime, frame, id, false, None, now).events);
    // An unconsumed direction moves focus.
    if !consumed && let Some(direction) = engine_focus::direction_of(key) {
        engine_focus::step(runtime, frame, direction);
    }
    events
}

/// The form's screen controller: what each screen event does to its values.
fn controller(
    runtime: &mut UiRuntime,
    frame: &EngineFrame,
    model: &ServerFormModel,
    event: &ScreenEvent,
    pressed: Option<&str>,
) -> Option<LocalFormAction> {
    match event {
        ScreenEvent::Button(button) => {
            // A pointer press answers on release over the control it went down on.
            let answers = if button.from == SELECT {
                !button.down && pressed == Some(button.key.as_str())
            } else {
                button.down && button.interacted
            };
            if button.id == "button.dropdown_exit" && button.down {
                close_dropdown(runtime, frame);
            }
            answers.then(|| mapped_action(model, button)).flatten()
        }
        ScreenEvent::Toggle {
            name,
            key,
            index,
            checked,
            ..
        } => {
            let index = (*index)?;
            let engine = runtime.server_forms_mut().engine_mut();
            let dropdown = frame
                .hits
                .iter()
                .any(|region| region.key == *key && region.kind == HitKind::Dropdown);
            if dropdown {
                engine.open_dropdown = checked.then_some(index);
            } else if name == "custom_dropdown_radio_toggle" {
                // Choosing an option answers the open dropdown and closes it.
                if *checked
                    && let Some(open) = engine.open_dropdown
                    && let Some(value) = engine.values.get_mut(open)
                {
                    *value = FormValue::Dropdown(index);
                    close_dropdown(runtime, frame);
                }
            } else if let Some(FormValue::Toggle(on)) = engine.values.get_mut(index) {
                *on = *checked;
            }
            None
        }
        ScreenEvent::Slider {
            index, value, step, ..
        } => {
            set_slider(runtime, model, (*index)?, *value, *step);
            None
        }
        ScreenEvent::TextEdit { index, text, .. } => {
            let engine = runtime.server_forms_mut().engine_mut();
            if let Some(FormValue::Text(value)) =
                index.and_then(|index| engine.values.get_mut(index))
            {
                value.clone_from(text);
            }
            None
        }
        ScreenEvent::Sound {
            name,
            volume,
            pitch,
        } => {
            crate::audio::ui_sound(name, *volume, *pitch);
            None
        }
        ScreenEvent::TextEditSelected { .. } => None,
    }
}

/// A button id (from a press or a `global` mapping) as a form answer.
fn mapped_action(model: &ServerFormModel, button: &ButtonEvent) -> Option<LocalFormAction> {
    let index = button.collection_index;
    match button.id.as_str() {
        "button.form_button_click" => {
            let index = index?;
            let ordinal = match model {
                // Decorations share the collection; answers count buttons only.
                ServerFormModel::ElementMenu(menu) => menu.elements
                    [..index.min(menu.elements.len())]
                    .iter()
                    .filter(|element| matches!(element, MenuElement::Button { .. }))
                    .count(),
                _ => index,
            };
            Some(LocalFormAction::SubmitButton(ordinal as u32))
        }
        "button.submit_custom_form" => Some(LocalFormAction::CustomElements),
        // NPC dialogue: a student button answers its action, exiting closes.
        "button.student_button" => Some(LocalFormAction::SubmitButton(index? as u32)),
        "button.exit_student" => Some(LocalFormAction::Dismiss),
        "popup_dialog.left_button" => Some(LocalFormAction::SubmitButton(0)),
        "popup_dialog.rightcancel_button" => Some(LocalFormAction::SubmitButton(1)),
        "button.menu_exit" | "popup_dialog.escape" => Some(LocalFormAction::Dismiss),
        _ => None,
    }
}

/// `button.dropdown_exit`: the controller closes its dropdown and the dropdown
/// toggles drop what they wrote, so their bound state shows again.
fn close_dropdown(runtime: &mut UiRuntime, frame: &EngineFrame) {
    let engine = runtime.server_forms_mut().engine_mut();
    engine.open_dropdown = None;
    for region in frame
        .hits
        .iter()
        .filter(|region| region.kind == HitKind::Dropdown)
    {
        engine.view.components.forget(&region.key);
    }
}

/// Continue a scrollbar drag while the button is held.
fn drag(runtime: &mut UiRuntime, frame: &EngineFrame, point: [f64; 2], held: bool) {
    if !held {
        return;
    }
    if let Some(FormDrag::ScrollBox { view, grab }) = runtime.server_forms().engine().drag.clone()
        && let Some(metrics) = frame.report.scrolls.get(&view)
    {
        let offset = metrics.offset_for_thumb(point[1] - grab);
        runtime
            .server_forms_mut()
            .engine_mut()
            .view
            .scroll
            .insert(view, offset);
    }
}

/// Start a scrollbar drag or page the track under a primary press.
fn press_scroll(runtime: &mut UiRuntime, frame: &EngineFrame, point: [f64; 2]) {
    let Some(region) = hit_test(&frame.hits, point).filter(|region| region.enabled) else {
        return;
    };
    match region.kind {
        HitKind::ScrollBox => {
            if let Some(view) = owning_view(frame, region)
                && let Some(thumb) = frame.report.scrolls.get(&view.key).and_then(|m| m.thumb)
            {
                runtime.server_forms_mut().engine_mut().drag = Some(FormDrag::ScrollBox {
                    view: view.key.clone(),
                    grab: point[1] - thumb[1],
                });
            }
        }
        HitKind::ScrollTrack => {
            if let Some(view) = owning_view(frame, region)
                && let Some(metrics) = frame.report.scrolls.get(&view.key)
            {
                let page = if metrics.thumb.is_some_and(|thumb| point[1] < thumb[1]) {
                    -metrics.viewport
                } else {
                    metrics.viewport
                };
                let key = view.key.clone();
                scroll_by(runtime, frame, &key, page);
            }
        }
        _ => {}
    }
}

fn edit_region<'a>(frame: &'a EngineFrame, key: &str) -> Option<&'a HitRegion> {
    frame
        .hits
        .iter()
        .find(|region| region.key == key && region.kind == HitKind::EditBox)
}

/// The innermost scroll view whose key prefixes `region`'s key.
fn owning_view<'a>(frame: &'a EngineFrame, region: &HitRegion) -> Option<&'a HitRegion> {
    frame
        .hits
        .iter()
        .filter(|view| view.kind == HitKind::ScrollView && region.key.starts_with(&view.key))
        .max_by_key(|view| view.key.len())
}

fn scroll_by(runtime: &mut UiRuntime, frame: &EngineFrame, key: &str, delta: f64) {
    let Some(metrics) = frame.report.scrolls.get(key) else {
        return;
    };
    let offset = (metrics.offset + delta).clamp(0.0, metrics.max_offset());
    runtime
        .server_forms_mut()
        .engine_mut()
        .view
        .scroll
        .insert(key.to_owned(), offset);
}

/// A slider event's `#slider_value` (a percentage, or a step index) as the element's value.
fn set_slider(
    runtime: &mut UiRuntime,
    model: &ServerFormModel,
    index: usize,
    value: f64,
    step: Option<usize>,
) {
    let ServerFormModel::Custom(form) = model else {
        return;
    };
    let value = match form.elements.get(index) {
        Some(CustomFormElement::Slider {
            min,
            max,
            step: size,
            ..
        }) => FormValue::Slider(slider_value_at(min.get(), max.get(), size.get(), value)),
        Some(CustomFormElement::StepSlider { steps, .. }) if !steps.is_empty() => {
            FormValue::Step(step.unwrap_or(value.max(0.0) as usize).min(steps.len() - 1))
        }
        _ => return,
    };
    if let Some(slot) = runtime
        .server_forms_mut()
        .engine_mut()
        .values
        .get_mut(index)
    {
        *slot = value;
    }
}
