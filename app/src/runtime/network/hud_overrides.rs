//! Server-pack overrides of the Java-styled HUD, read from the merged `ui/` files.

use std::sync::Arc;

use resource_pack::LayeredPackView;
use serde_json::{Map, Value};

use super::resource_packs::parse_pack_json;
use crate::ui_runtime::presentation::SessionHudOverrides;

const SCOREBOARD_FILE: &str = "ui/scoreboards.json";
const SCORE_ELEMENT: &str = "scoreboard_sidebar_score";

/// The HUD elements the stack's UI files hide, or `None` when it touches none.
pub(super) fn compile_hud_overrides(view: &LayeredPackView) -> Option<Arc<SessionHudOverrides>> {
    let overrides = SessionHudOverrides {
        hide_sidebar_scores: merged_element(view, SCOREBOARD_FILE, "scoreboard", SCORE_ELEMENT)
            .is_some_and(|element| is_hidden(&element)),
    };
    (overrides != SessionHudOverrides::default()).then(|| Arc::new(overrides))
}

/// The named element's properties merged across layers (a higher pack wins per key), or
/// `None` when no layer defines it; the `@base` suffix of a definition key is ignored.
fn merged_element(
    view: &LayeredPackView,
    path: &str,
    namespace: &str,
    name: &str,
) -> Option<Map<String, Value>> {
    let mut merged: Option<Map<String, Value>> = None;
    for layer in view.read_layers(path) {
        let Some(Value::Object(root)) = parse_pack_json(&layer) else {
            continue;
        };
        if root.get("namespace").and_then(Value::as_str) != Some(namespace) {
            continue;
        }
        for (key, value) in root {
            if key.split('@').next() == Some(name)
                && let Value::Object(properties) = value
            {
                merged.get_or_insert_with(Map::new).extend(properties);
            }
        }
    }
    merged
}

/// Whether the element draws nothing: invisible, ignored, zero-width, fully transparent or
/// empty text. Variable-driven values (`$x`, bindings) are not resolved and count as shown.
fn is_hidden(element: &Map<String, Value>) -> bool {
    let is_zero = |value: &Value| match value {
        Value::Number(number) => number.as_f64() == Some(0.0),
        Value::String(text) => matches!(text.trim(), "0" | "0px" | "0%"),
        _ => false,
    };
    element.get("visible") == Some(&Value::Bool(false))
        || element.get("ignored") == Some(&Value::Bool(true))
        || element.get("text").and_then(Value::as_str) == Some("")
        || ["alpha", "locked_alpha"]
            .iter()
            .any(|key| element.get(*key).is_some_and(is_zero))
        || element
            .get("size")
            .and_then(|size| size.get(0))
            .is_some_and(is_zero)
}

#[cfg(test)]
mod tests;
