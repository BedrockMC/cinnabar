//! Layout parity regressions against the client's layout rules, one per audited
//! row (`G`, `S`, `D` ids). Trees are written in pack syntax and laid out in a
//! 100x100 screen unless a test says otherwise.

use std::collections::BTreeMap;

use json_ui::{
    LaidOut, LayoutEnv, ResolvedControl, TextMeasure, TextureMeta, TextureSource, layout,
};

use serde_json::{Value, json};

struct MonoText;

impl TextMeasure for MonoText {
    fn extent(&self, text: &str) -> [f64; 2] {
        [text.chars().count() as f64 * 6.0, 10.0]
    }
}

/// Every texture is 40x20.
struct Textures;

impl TextureSource for Textures {
    fn texture(&self, _path: &str) -> Option<TextureMeta> {
        Some(TextureMeta {
            base_size: [40.0, 20.0],
            nineslice: None,
        })
    }
}

fn env() -> LayoutEnv<'static> {
    LayoutEnv {
        text: &MonoText,
        textures: &Textures,
    }
}

/// A control from pack syntax: `{"type": ..., "controls": [{"name": {...}}]}`.
fn control(name: &str, body: Value) -> ResolvedControl {
    let Value::Object(mut map) = body else {
        panic!("{name}: not an object");
    };
    let control_type = map
        .remove("type")
        .and_then(|kind| kind.as_str().map(str::to_owned));
    let children = match map.remove("controls") {
        Some(Value::Array(items)) => items
            .into_iter()
            .map(|item| {
                let Value::Object(entry) = item else {
                    panic!("{name}: child not an object");
                };
                let (child, body) = entry.into_iter().next().expect("named child");
                control(&child, body)
            })
            .collect(),
        _ => Vec::new(),
    };
    ResolvedControl {
        name: name.to_owned(),
        control_type,
        base: None,
        unresolved_base: None,
        properties: map.into_iter().collect::<BTreeMap<_, _>>(),
        children,
        factory: None,
    }
}

/// `controls` under a 100x100 top-left root.
fn screen(controls: Value) -> ResolvedControl {
    control(
        "root",
        json!({
            "type": "panel",
            "size": [100, 100],
            "anchor_from": "top_left",
            "anchor_to": "top_left",
            "controls": controls,
        }),
    )
}

fn find<'a>(node: &'a LaidOut<'a>, name: &str) -> &'a LaidOut<'a> {
    if node.control.name == name {
        return node;
    }
    node.children
        .iter()
        .find_map(|child| {
            let found = find(child, name);
            (found.control.name == name).then_some(found)
        })
        .unwrap_or(node)
}

/// `[x, y, w, h]` of the control named `name`.
fn rect(root: &ResolvedControl, name: &str) -> [f64; 4] {
    let laid = layout(root, [100.0, 100.0], &env());
    let node = find(&laid, name);
    assert_eq!(node.control.name, name, "{name} not laid out");
    [node.rect.x, node.rect.y, node.rect.w, node.rect.h]
}

fn size(root: &ResolvedControl, name: &str) -> [f64; 2] {
    let [_, _, w, h] = rect(root, name);
    [w, h]
}

fn top_left(body: Value) -> Value {
    let mut body = body;
    body["anchor_from"] = json!("top_left");
    body["anchor_to"] = json!("top_left");
    body
}

// G03: upper-case units and a repeated sign parse as the client normalizes them.
#[test]
fn g03_normalized_expressions() {
    let root = screen(json!([
        { "a": { "type": "panel", "size": ["20PX", 10] } },
        { "b": { "type": "panel", "size": ["100% + -4px", 10] } },
    ]));
    assert_eq!(size(&root, "a"), [20.0, 10.0]);
    assert_eq!(size(&root, "b"), [96.0, 10.0]);
}

// G06: `%cm` is the largest child whatever its coefficient.
#[test]
fn g06_child_max_ignores_its_coefficient() {
    let root = screen(json!([{ "p": {
        "type": "panel", "size": ["50%cm", 10],
        "controls": [{ "c": { "type": "panel", "size": [40, 10] } }],
    } }]));
    assert_eq!(size(&root, "p"), [40.0, 10.0]);
}

// G11: a `default` offset is no offset.
#[test]
fn g11_default_offset_is_none() {
    let root = screen(json!([{ "p": top_left(json!({
        "type": "panel", "size": [20, 10], "offset": ["default", 0],
    })) }]));
    assert_eq!(rect(&root, "p")[0], 0.0);
}

// G12: `default`/`fill` bounds install no rule.
#[test]
fn g12_keyword_bounds_are_unbounded() {
    let root = screen(json!([
        { "a": { "type": "panel", "size": [20, 10], "min_size": ["fill", 0] } },
        { "b": { "type": "panel", "size": [20, 10], "min_size": ["default", 0] } },
        { "c": { "type": "panel", "size": [120, 10], "max_size": ["default", 100] } },
    ]));
    assert_eq!(size(&root, "a")[0], 20.0);
    assert_eq!(size(&root, "b")[0], 20.0);
    assert_eq!(size(&root, "c")[0], 120.0);
}
