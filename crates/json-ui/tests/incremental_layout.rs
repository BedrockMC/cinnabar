//! Dirty subtree layouts must match a fresh layout after every update.

use std::{cell::Cell, collections::BTreeMap};

use json_ui::{
    LayoutEnv, MeasureCache, ResolvedControl, TextMeasure, TextureMeta, TextureSource, ViewState,
    render_bound, render_bound_cached,
};
use serde_json::json;

#[derive(Default)]
struct Text(Cell<usize>);

impl TextMeasure for Text {
    fn extent(&self, text: &str) -> [f64; 2] {
        self.0.set(self.0.get() + 1);
        [text.len() as f64 * 6.0, 9.0]
    }
}

struct Textures;

impl TextureSource for Textures {
    fn texture(&self, _: &str) -> Option<TextureMeta> {
        None
    }
}

/// A vertical stack with one changing label and one independent panel.
fn tree(text: &str, count: usize) -> ResolvedControl {
    let label = |name: &str, text: &str| ResolvedControl {
        name: name.into(),
        control_type: Some("label".into()),
        properties: BTreeMap::from([
            ("text".into(), json!(text)),
            ("size".into(), json!(["default", "default"])),
        ]),
        children: Vec::new(),
        base: None,
        unresolved_base: None,
        factory: None,
    };
    let mut root = label("root", "");
    root.control_type = Some("stack_panel".into());
    root.properties = BTreeMap::from([
        ("orientation".into(), json!("vertical")),
        ("size".into(), json!(["100%cm", "100%c"])),
    ]);
    root.children = (0..count)
        .map(|index| label(&index.to_string(), if index == 0 { text } else { "fixed" }))
        .collect();
    root
}

#[test]
fn changed_labels_and_collection_shapes_match_cold_layouts() {
    let text = Text::default();
    let env = LayoutEnv {
        text: &text,
        textures: &Textures,
    };
    let mut cache = MeasureCache::default();
    let state = ViewState::default();
    let mut rendered =
        render_bound_cached(tree("one", 3), [480.0, 270.0], &env, &state, &mut cache);
    for (label, count) in [
        ("much longer text", 3),
        ("short", 2),
        ("more", 5),
        ("last", 5),
    ] {
        let next = tree(label, count);
        cache.update_tree(&mut rendered.bound, next.clone());
        rendered = render_bound_cached(rendered.bound, [480.0, 270.0], &env, &state, &mut cache);
        let cold = render_bound(next, [480.0, 270.0], &env, &state);
        assert_eq!(rendered.nodes, cold.nodes);
        assert_eq!(rendered.hits, cold.hits);
        assert_eq!(rendered.report, cold.report);
    }
}

#[test]
fn unchanged_tree_keeps_label_measurements() {
    let text = Text::default();
    let env = LayoutEnv {
        text: &text,
        textures: &Textures,
    };
    let mut cache = MeasureCache::default();
    let state = ViewState::default();
    let mut rendered =
        render_bound_cached(tree("one", 3), [480.0, 270.0], &env, &state, &mut cache);
    text.0.set(0);
    cache.update_tree(&mut rendered.bound, tree("one", 3));
    render_bound_cached(rendered.bound, [480.0, 270.0], &env, &state, &mut cache);
    assert_eq!(text.0.get(), 0);
}
