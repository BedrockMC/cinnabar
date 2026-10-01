//! Which rect a clipping control (`clips_children`) clips its children to.

use serde_json::Value;

use super::Rect;
use crate::tree::ResolvedControl;

/// The rect a clipping control clips its children to: its own, moved by `clip_offset`.
pub(super) fn clip_rect(control: &ResolvedControl, rect: Rect) -> Rect {
    let offset = |index: usize| {
        control
            .properties
            .get("clip_offset")
            .and_then(Value::as_array)
            .and_then(|items| items.get(index)?.as_f64())
            .unwrap_or(0.0)
    };
    Rect::new(rect.x + offset(0), rect.y + offset(1), rect.w, rect.h)
}

pub(super) fn clip_children(control: &ResolvedControl) -> bool {
    ["clips_children", "clip_children"]
        .iter()
        .any(|key| matches!(control.properties.get(*key), Some(Value::Bool(true))))
}
