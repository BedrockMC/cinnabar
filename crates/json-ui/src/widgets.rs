//! Engine-driven control behaviour that the templates only name: which state
//! child of a button/toggle/edit box/slider shows, where a slider's box sits and
//! how much of its progress bar is revealed. The templates supply the child
//! names (`default_control`, `checked_hover_control`, …); the state comes from
//! the bound `#` values and the caller's [`ViewState`].

use serde_json::Value;

use crate::layout::Rect;
use crate::state::ViewState;
use crate::tree::ResolvedControl;

/// Property names that name a state child, per control type.
const BUTTON_STATES: [&str; 4] = [
    "default_control",
    "hover_control",
    "pressed_control",
    "locked_control",
];
const TOGGLE_STATES: [&str; 8] = [
    "unchecked_control",
    "checked_control",
    "unchecked_hover_control",
    "checked_hover_control",
    "unchecked_locked_control",
    "checked_locked_control",
    "unchecked_locked_hover_control",
    "checked_locked_hover_control",
];

fn prop_str<'a>(control: &'a ResolvedControl, key: &str) -> Option<&'a str> {
    control
        .properties
        .get(key)
        .and_then(Value::as_str)
        .filter(|name| !name.is_empty())
}

/// A bound boolean: literal bool, `"true"`/`"false"`, or `None`.
pub(crate) fn bound_bool(control: &ResolvedControl, key: &str) -> Option<bool> {
    match control.properties.get(key)? {
        Value::Bool(flag) => Some(*flag),
        Value::String(text) if text == "true" => Some(true),
        Value::String(text) if text == "false" => Some(false),
        _ => None,
    }
}

pub(crate) fn bound_number(control: &ResolvedControl, key: &str) -> Option<f64> {
    match control.properties.get(key)? {
        Value::Number(number) => number.as_f64(),
        Value::String(text) => text.parse().ok(),
        _ => None,
    }
}

/// Whether the control accepts input: `enabled`/`#enabled` false locks it.
pub(crate) fn enabled(control: &ResolvedControl) -> bool {
    bound_bool(control, "#enabled")
        .or_else(|| bound_bool(control, "enabled"))
        .unwrap_or(true)
}

/// A toggle's checked state: the bound `#toggle_state`, else its default.
pub(crate) fn toggle_checked(control: &ResolvedControl) -> bool {
    bound_bool(control, "#toggle_state")
        .or_else(|| bound_bool(control, "toggle_default_state"))
        .unwrap_or(false)
}

/// Names of state children to hide under `control` this frame; every other child
/// keeps its own visibility. Non-stateful controls hide nothing.
pub(crate) fn hidden_state_children(
    control: &ResolvedControl,
    key: &str,
    state: &ViewState,
) -> Vec<String> {
    hidden_under(
        control,
        state.is_hovered(key),
        state.is_pressed(key),
        state.is_focused(key),
    )
}

/// State children hidden at rest (no hover, press or focus): the client hides
/// them, so they add nothing to their parent's `%c`/`%cm`.
pub(crate) fn rest_hidden_children(control: &ResolvedControl) -> Vec<String> {
    hidden_under(control, false, false, false)
}

/// Per state child of a stateful control, the interaction states it shows under,
/// as a mask over [`state_index`]; empty for controls without state children.
pub(crate) fn state_child_masks(control: &ResolvedControl) -> Vec<(String, u8)> {
    let names: &[&str] = match control.control_type.as_deref().unwrap_or("") {
        "button" | "edit_box" | "slider_box" | "slider" => &BUTTON_STATES,
        "toggle" | "dropdown" => &TOGGLE_STATES,
        _ => return Vec::new(),
    };
    let mut masks: Vec<(String, u8)> = names
        .iter()
        .filter_map(|property| prop_str(control, property))
        .map(|name| (name.to_owned(), 0))
        .collect();
    masks.dedup();
    for index in 0..8u8 {
        let hidden = hidden_under(control, index & 1 != 0, index & 2 != 0, index & 4 != 0);
        for (name, mask) in &mut masks {
            if !hidden.contains(name) {
                *mask |= 1 << index;
            }
        }
    }
    masks
}

/// The bit a control's interaction state takes in a [`state_child_masks`] mask.
pub(crate) fn state_index(state: &ViewState, key: &str) -> u8 {
    u8::from(state.is_hovered(key))
        | (u8::from(state.is_pressed(key)) << 1)
        | (u8::from(state.is_focused(key)) << 2)
}

fn hidden_under(
    control: &ResolvedControl,
    hovered: bool,
    pressed: bool,
    focused: bool,
) -> Vec<String> {
    let kind = control.control_type.as_deref().unwrap_or("");
    let locked = !enabled(control);
    let (names, shown): (&[&str], &str) = match kind {
        "button" | "edit_box" | "slider_box" => {
            let shown = if locked {
                "locked_control"
            } else if pressed || (kind == "edit_box" && focused) {
                "pressed_control"
            } else if hovered {
                "hover_control"
            } else {
                "default_control"
            };
            (&BUTTON_STATES[..], shown)
        }
        "toggle" | "dropdown" => {
            let checked = toggle_checked(control);
            let index = usize::from(checked) + 2 * usize::from(hovered) + 4 * usize::from(locked);
            (&TOGGLE_STATES[..], TOGGLE_STATES[index])
        }
        "slider" => {
            let shown = if hovered || pressed {
                "hover_control"
            } else {
                "default_control"
            };
            (&BUTTON_STATES[..2], shown)
        }
        _ => return Vec::new(),
    };
    let mut hidden = Vec::new();
    for property in names {
        let Some(name) = prop_str(control, property) else {
            continue;
        };
        // A state that falls back to the same child (e.g. pressed == hover) keeps it.
        if *property != shown && Some(name) != prop_str(control, shown) {
            hidden.push(name.to_owned());
        }
    }
    // An unset locked child means "no locked look": fall back to the default child.
    if prop_str(control, shown).is_none()
        && let Some(default) = prop_str(control, "default_control")
    {
        hidden.retain(|name| name != default);
    }
    hidden
}

/// A slider's normalized position `0..=1`: a step slider's `#slider_value` is the
/// step index over `#slider_steps`, a continuous slider's value is already a
/// fraction.
pub(crate) fn slider_fraction(control: &ResolvedControl) -> Option<f64> {
    if control.control_type.as_deref() != Some("slider") {
        return None;
    }
    let value = bound_number(control, "#slider_value").unwrap_or(0.0);
    let steps = bound_number(control, "#slider_steps")
        .or_else(|| bound_number(control, "slider_steps"))
        .unwrap_or(1.0);
    let fraction = if steps > 1.0 {
        value / (steps - 1.0)
    } else {
        value
    };
    Some(fraction.clamp(0.0, 1.0))
}

/// The slider box's rect: its centre travels the full track width.
pub(crate) fn slider_box_rect(track: Rect, box_rect: Rect, fraction: f64) -> Rect {
    Rect::new(
        track.x + track.w * fraction - box_rect.w * 0.5,
        box_rect.y,
        box_rect.w,
        box_rect.h,
    )
}

pub(crate) fn slider_names(control: &ResolvedControl) -> [Option<String>; 3] {
    [
        prop_str(control, "slider_box_control").map(str::to_owned),
        prop_str(control, "progress_control").map(str::to_owned),
        prop_str(control, "progress_hover_control").map(str::to_owned),
    ]
}

/// A panel holding a `dropdown` toggle: the toggle's `dropdown_area` (the
/// ancestor its content lays out in) and the content child's name.
pub(crate) fn dropdown_area(control: &ResolvedControl) -> Option<(String, String)> {
    control.children.iter().find_map(|child| {
        if child.control_type.as_deref() != Some("dropdown") {
            return None;
        }
        let area = prop_str(child, "dropdown_area")?;
        let content = prop_str(child, "dropdown_content_control").unwrap_or("dropdown_content");
        Some((area.to_owned(), content.to_owned()))
    })
}
