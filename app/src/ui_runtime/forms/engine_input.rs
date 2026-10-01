//! Input for a form drawn by the JSON-UI engine. Pointer, wheel, and keyboard
//! events resolve against the last frame's hit regions; the template's mapping
//! names (`button.form_button_click`, `button.submit_custom_form`,
//! `button.menu_exit`, `popup_dialog.*`, toggle/slider/dropdown/edit-box names)
//! decide what happens. Buttons fire on release over the pressed control.

use bevy::input::{ButtonInput, keyboard::KeyCode, mouse::MouseScrollUnit};
use json_ui::{HitKind, HitRegion, focus_order, hit_test, wheel_target};
use protocol::{CustomFormElement, MenuElement, ServerFormModel};
use ui::{ChatClipboard, UiPoint};

use super::values::{EngineFrame, FormDrag, slider_value_at};
use super::{FormValue, LocalFormAction};
use crate::ui_runtime::{PlatformClipboard, UiRuntime};

/// Longest paste accepted into an edit box, before its own `max_length`.
const MAX_PASTE_BYTES: usize = 4096;
/// The custom input template's `max_length` when a region reports none.
const DEFAULT_INPUT_LENGTH: usize = 100;

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
}

pub(super) fn drive(runtime: &mut UiRuntime, frame: &EngineFrame, input: EngineInput<'_>) {
    let Some(entry) = runtime.server_forms().active() else {
        return;
    };
    let identity = entry.identity;
    let model = entry.model.clone();
    // `jump_to_bottom_on_update` compares each view's maximum with last frame's.
    runtime
        .server_forms_mut()
        .engine_mut()
        .view
        .remember(&frame.report);
    let point = input.cursor.map(|cursor| frame.to_virtual(cursor));
    let hovered = point.and_then(|point| hit_test(&frame.hits, point));
    let control = input.keys.pressed(KeyCode::ControlLeft)
        || input.keys.pressed(KeyCode::ControlRight)
        || input.keys.pressed(KeyCode::SuperLeft)
        || input.keys.pressed(KeyCode::SuperRight);

    {
        let engine = runtime.server_forms_mut().engine_mut();
        engine.view.hovered = hovered
            .filter(|region| region.kind.focusable() && region.enabled)
            .map(|region| region.key.clone())
            .or_else(|| engine.view.focused.clone());
    }
    if let Some(point) = point {
        drag(runtime, frame, &model, point, input.pointer.held);
    }
    let mut action = None;
    if input.pointer.pressed
        && let Some(point) = point
    {
        if let Some(sound) = hovered
            .filter(|region| region.enabled)
            .and_then(|region| region.sound.as_ref())
        {
            crate::audio::ui_sound(sound);
        }
        press(runtime, frame, &model, hovered, point);
    }
    if input.pointer.released {
        let engine = runtime.server_forms_mut().engine_mut();
        let pressed = engine.view.pressed.take();
        engine.drag = None;
        if let Some(region) = hovered
            && pressed.as_deref() == Some(region.key.as_str())
        {
            action = activate(runtime, &model, region);
        }
    }
    for (notches, unit) in &input.wheel {
        if let Some(point) = point
            && let Some(view) = wheel_target(&frame.hits, &frame.report, point)
            && let Some(metrics) = frame.report.scrolls.get(&view.key)
        {
            let offset = match unit {
                MouseScrollUnit::Line => metrics.wheel_target(f64::from(*notches)),
                MouseScrollUnit::Pixel => (metrics.offset - f64::from(notches / frame.scale))
                    .clamp(0.0, metrics.max_offset()),
            };
            set_scroll(runtime, &view.key, offset);
        }
    }
    for (key, text) in input.typed {
        if let Some(found) = keyboard(runtime, frame, &model, key, text.as_deref(), control) {
            action = Some(found);
        }
    }
    if let Some(action) = action {
        let _ = runtime.respond_to_server_form(identity, action);
    }
}

/// Continue a slider or scrollbar drag while the button is held.
fn drag(
    runtime: &mut UiRuntime,
    frame: &EngineFrame,
    model: &ServerFormModel,
    point: [f64; 2],
    held: bool,
) {
    if !held {
        return;
    }
    match runtime.server_forms().engine().drag.clone() {
        Some(FormDrag::Slider(index)) => {
            if let Some(region) = element_region(frame, HitKind::Slider, index) {
                set_slider(runtime, model, index, region.fraction_at(point[0]));
            }
        }
        Some(FormDrag::ScrollBox { view, last }) => {
            if let Some(metrics) = frame.report.scrolls.get(&view) {
                let along = point[usize::from(!metrics.horizontal)];
                let offset = metrics.thumb_drag_target(along - last);
                let engine = runtime.server_forms_mut().engine_mut();
                engine.view.scroll.insert(view.clone(), offset);
                engine.drag = Some(FormDrag::ScrollBox { view, last: along });
            }
        }
        Some(FormDrag::Control { key, last }) => {
            let axes = frame
                .hits
                .iter()
                .find(|region| region.key == key)
                .map_or([false; 2], |region| region.drag_axes);
            let engine = runtime.server_forms_mut().engine_mut();
            let moved = engine.view.drags.entry(key.clone()).or_insert([0.0; 2]);
            for axis in 0..2 {
                if axes[axis] {
                    moved[axis] += point[axis] - last[axis];
                }
            }
            engine.drag = Some(FormDrag::Control { key, last: point });
        }
        None => {}
    }
}

fn press(
    runtime: &mut UiRuntime,
    frame: &EngineFrame,
    model: &ServerFormModel,
    hovered: Option<&HitRegion>,
    point: [f64; 2],
) {
    let engine = runtime.server_forms_mut().engine_mut();
    let on_edit_box = hovered.is_some_and(|region| region.kind == HitKind::EditBox);
    if !on_edit_box {
        engine.editing = None;
    }
    let on_radio = hovered.is_some_and(|region| {
        region.control_name.as_deref() == Some("custom_dropdown_radio_toggle")
    });
    let on_dropdown = hovered.is_some_and(|region| region.kind == HitKind::Dropdown);
    // Any click that is not choosing an option dismisses an open dropdown.
    if !on_radio && !on_dropdown {
        engine.open_dropdown = None;
    }
    let Some(region) = hovered.filter(|region| region.enabled) else {
        return;
    };
    match region.kind {
        HitKind::ScrollBox => {
            if let Some(view) = owning_view(frame, region)
                && let Some(metrics) = frame.report.scrolls.get(&view.key)
            {
                engine.drag = Some(FormDrag::ScrollBox {
                    view: view.key.clone(),
                    last: point[usize::from(!metrics.horizontal)],
                });
            }
        }
        // A track press jumps only when it routes to the view's track button.
        HitKind::ScrollTrack => {
            if let Some(view) = owning_view(frame, region)
                && let Some(metrics) = frame.report.scrolls.get(&view.key)
                && metrics.track_clicks
            {
                let key = view.key.clone();
                set_scroll(runtime, &key, metrics.track_target(point));
            }
        }
        HitKind::Draggable => {
            engine.drag = Some(FormDrag::Control {
                key: region.key.clone(),
                last: point,
            });
        }
        HitKind::Slider => {
            if let Some(index) = region.collection_index {
                engine.drag = Some(FormDrag::Slider(index));
                engine.view.pressed = Some(region.key.clone());
                set_slider(runtime, model, index, region.fraction_at(point[0]));
            }
        }
        HitKind::EditBox => {
            engine.editing = region.collection_index;
            engine.view.focused = Some(region.key.clone());
        }
        HitKind::Button | HitKind::Toggle | HitKind::Dropdown => {
            engine.view.pressed = Some(region.key.clone());
        }
        HitKind::ScrollView | HitKind::Modal | HitKind::Panel | HitKind::Custom => {}
    }
}

/// What releasing over (or keyboard-activating) `region` does.
fn activate(
    runtime: &mut UiRuntime,
    model: &ServerFormModel,
    region: &HitRegion,
) -> Option<LocalFormAction> {
    match region.kind {
        HitKind::Button => region
            .pressed
            .as_deref()
            .and_then(|pressed| mapped_action(runtime, model, pressed, region.collection_index)),
        HitKind::Toggle => {
            let index = region.collection_index?;
            let engine = runtime.server_forms_mut().engine_mut();
            match region.control_name.as_deref() {
                Some("custom_dropdown_radio_toggle") => {
                    let dropdown = engine.open_dropdown.take()?;
                    if let Some(value) = engine.values.get_mut(dropdown) {
                        *value = FormValue::Dropdown(index);
                    }
                }
                _ => {
                    if let Some(FormValue::Toggle(on)) = engine.values.get_mut(index) {
                        *on = !*on;
                    }
                }
            }
            None
        }
        HitKind::Dropdown => {
            let index = region.collection_index?;
            let engine = runtime.server_forms_mut().engine_mut();
            engine.open_dropdown = (engine.open_dropdown != Some(index)).then_some(index);
            None
        }
        _ => None,
    }
}

/// A button id (from a press or a `global` mapping) as a form answer.
fn mapped_action(
    runtime: &mut UiRuntime,
    model: &ServerFormModel,
    pressed: &str,
    index: Option<usize>,
) -> Option<LocalFormAction> {
    match pressed {
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
        "button.dropdown_exit" => {
            runtime.server_forms_mut().engine_mut().open_dropdown = None;
            None
        }
        _ => None,
    }
}

fn keyboard(
    runtime: &mut UiRuntime,
    frame: &EngineFrame,
    model: &ServerFormModel,
    key: KeyCode,
    text: Option<&str>,
    control: bool,
) -> Option<LocalFormAction> {
    if let Some(index) = runtime.server_forms().engine().editing {
        edit_text(runtime, frame, index, key, text, control);
        return None;
    }
    match key {
        KeyCode::Escape => {
            let target = frame.cancel_target.clone()?;
            mapped_action(runtime, model, &target, None)
        }
        KeyCode::Tab | KeyCode::ArrowDown | KeyCode::ArrowUp => {
            let backwards = key == KeyCode::ArrowUp;
            move_focus(runtime, frame, backwards);
            None
        }
        KeyCode::ArrowLeft | KeyCode::ArrowRight => {
            let focused = focused_region(runtime, frame)?;
            let index = focused.collection_index?;
            if focused.kind == HitKind::Slider {
                step_slider(runtime, model, index, key == KeyCode::ArrowRight);
            }
            None
        }
        KeyCode::Enter | KeyCode::NumpadEnter | KeyCode::Space => {
            let focused = focused_region(runtime, frame)?.clone();
            if focused.kind == HitKind::EditBox {
                runtime.server_forms_mut().engine_mut().editing = focused.collection_index;
                return None;
            }
            activate(runtime, model, &focused)
        }
        _ => None,
    }
}

fn edit_text(
    runtime: &mut UiRuntime,
    frame: &EngineFrame,
    index: usize,
    key: KeyCode,
    text: Option<&str>,
    control: bool,
) {
    let limit = element_region(frame, HitKind::EditBox, index)
        .and_then(|region| region.max_length)
        .unwrap_or(DEFAULT_INPUT_LENGTH);
    let pasted = (control && key == KeyCode::KeyV)
        .then(|| {
            PlatformClipboard
                .read_text_bounded(MAX_PASTE_BYTES)
                .ok()
                .flatten()
        })
        .flatten();
    let engine = runtime.server_forms_mut().engine_mut();
    let Some(FormValue::Text(value)) = engine.values.get_mut(index) else {
        engine.editing = None;
        return;
    };
    let mut insert = |addition: &str| {
        for character in addition.chars().filter(|c| !c.is_control()) {
            if value.chars().count() >= limit {
                break;
            }
            value.push(character);
        }
    };
    match key {
        KeyCode::Enter | KeyCode::NumpadEnter | KeyCode::Escape | KeyCode::Tab => {
            engine.editing = None;
        }
        KeyCode::Backspace => {
            value.pop();
        }
        _ if pasted.is_some() => insert(pasted.as_deref().unwrap_or("")),
        _ if !control => {
            if let Some(text) = text {
                insert(text);
            }
        }
        _ => {}
    }
}

fn move_focus(runtime: &mut UiRuntime, frame: &EngineFrame, backwards: bool) {
    let order = focus_order(&frame.hits);
    if order.is_empty() {
        return;
    }
    let engine = runtime.server_forms_mut().engine_mut();
    let current = engine
        .view
        .focused
        .as_deref()
        .and_then(|key| order.iter().position(|region| region.key == key));
    let next = match (current, backwards) {
        (None, false) => 0,
        (None, true) => order.len() - 1,
        (Some(at), false) => (at + 1) % order.len(),
        (Some(at), true) => (at + order.len() - 1) % order.len(),
    };
    engine.view.focused = Some(order[next].key.clone());
    engine.view.hovered = engine.view.focused.clone();
    // Keep the focused control inside its scroll view.
    let region = order[next];
    if let Some(view) = owning_view(frame, region)
        && let Some(metrics) = frame.report.scrolls.get(&view.key)
    {
        let offset = metrics.offset_revealing(region.rect.y, region.rect.y + region.rect.h);
        engine.view.scroll.insert(view.key.clone(), offset);
    }
}

fn focused_region<'a>(runtime: &UiRuntime, frame: &'a EngineFrame) -> Option<&'a HitRegion> {
    let key = runtime.server_forms().engine().view.focused.as_deref()?;
    frame.hits.iter().find(|region| region.key == key)
}

fn element_region(frame: &EngineFrame, kind: HitKind, index: usize) -> Option<&HitRegion> {
    frame
        .hits
        .iter()
        .find(|region| region.kind == kind && region.collection_index == Some(index))
}

/// The innermost scroll view whose key prefixes `region`'s key.
fn owning_view<'a>(frame: &'a EngineFrame, region: &HitRegion) -> Option<&'a HitRegion> {
    frame
        .hits
        .iter()
        .filter(|view| view.kind == HitKind::ScrollView && region.key.starts_with(&view.key))
        .max_by_key(|view| view.key.len())
}

fn set_scroll(runtime: &mut UiRuntime, key: &str, offset: f64) {
    runtime
        .server_forms_mut()
        .engine_mut()
        .view
        .scroll
        .insert(key.to_owned(), offset);
}

fn set_slider(runtime: &mut UiRuntime, model: &ServerFormModel, index: usize, fraction: f64) {
    let ServerFormModel::Custom(form) = model else {
        return;
    };
    let value = match form.elements.get(index) {
        Some(CustomFormElement::Slider { min, max, step, .. }) => {
            FormValue::Slider(slider_value_at(min.get(), max.get(), step.get(), fraction))
        }
        Some(CustomFormElement::StepSlider { steps, .. }) if !steps.is_empty() => {
            FormValue::Step((fraction * (steps.len() - 1) as f64).round() as usize)
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

fn step_slider(runtime: &mut UiRuntime, model: &ServerFormModel, index: usize, up: bool) {
    let ServerFormModel::Custom(form) = model else {
        return;
    };
    let current = runtime.server_forms().engine().values.get(index).cloned();
    let value = match (form.elements.get(index), current) {
        (
            Some(CustomFormElement::Slider { min, max, step, .. }),
            Some(FormValue::Slider(value)),
        ) => {
            let delta = if up { step.get() } else { -step.get() };
            FormValue::Slider((value + delta).clamp(min.get(), max.get()))
        }
        (Some(CustomFormElement::StepSlider { steps, .. }), Some(FormValue::Step(at))) => {
            let last = steps.len().saturating_sub(1);
            FormValue::Step(if up {
                (at + 1).min(last)
            } else {
                at.saturating_sub(1)
            })
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
