//! Preserve control addresses while invalidating changed measurements.

use std::collections::HashSet;

use crate::ResolvedControl;

/// Apply a fresh binding, marking changed controls and their ancestors.
pub(super) fn update(
    tree: &mut ResolvedControl,
    next: ResolvedControl,
    dirty: &mut HashSet<usize>,
) -> bool {
    let same_children = tree.children.len() == next.children.len()
        && tree.children.iter().zip(&next.children).all(|(a, b)| {
            a.name == b.name
                && a.properties.get("collection_index") == b.properties.get("collection_index")
        });
    let changed = tree.name != next.name
        || tree.control_type != next.control_type
        || tree.properties != next.properties
        || tree.base != next.base
        || tree.unresolved_base != next.unresolved_base
        || tree.factory != next.factory;
    let mut changed = changed || !same_children;
    if same_children {
        for (child, next) in tree.children.iter_mut().zip(next.children) {
            changed |= update(child, next, dirty);
        }
    } else {
        for child in &tree.children {
            mark_subtree(child, dirty);
        }
        tree.children = next.children;
    }
    tree.name = next.name;
    tree.control_type = next.control_type;
    tree.properties = next.properties;
    tree.base = next.base;
    tree.unresolved_base = next.unresolved_base;
    tree.factory = next.factory;
    if changed {
        dirty.insert(std::ptr::from_ref(tree).addr());
    }
    changed
}

/// Remove every address before replacing a child allocation.
fn mark_subtree(tree: &ResolvedControl, dirty: &mut HashSet<usize>) {
    dirty.insert(std::ptr::from_ref(tree).addr());
    for child in &tree.children {
        mark_subtree(child, dirty);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{LayoutEnv, MeasureCache, TextMeasure, TextureMeta, TextureSource, ViewState};
    use serde_json::{Value, json};
    use std::cell::Cell;

    struct Metrics(Cell<usize>);
    impl TextMeasure for Metrics {
        fn extent(&self, _: &str) -> [f64; 2] {
            self.0.set(self.0.get() + 1);
            [20.0, 9.0]
        }
    }
    impl TextureSource for Metrics {
        fn texture(&self, _: &str) -> Option<TextureMeta> {
            None
        }
    }

    /// Build a small control without resolving a pack.
    fn node(
        name: &str,
        kind: &str,
        properties: Value,
        children: Vec<ResolvedControl>,
    ) -> ResolvedControl {
        ResolvedControl {
            name: name.into(),
            control_type: Some(kind.into()),
            properties: properties
                .as_object()
                .unwrap()
                .iter()
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect(),
            children,
            base: None,
            unresolved_base: None,
            factory: None,
        }
    }

    #[test]
    fn moving_the_root_keeps_fitting_scrollbar_measurements() {
        let label = node("content", "label", json!({"text":"short"}), vec![]);
        let viewport = node("viewport", "panel", json!({"size":[100,100]}), vec![label]);
        let track = node(
            "track",
            "panel",
            json!({"size":[4,100]}),
            vec![node(
                "box",
                "scrollbar_box",
                json!({"size":[4,10],"draggable":"vertical"}),
                vec![],
            )],
        );
        let view = node(
            "view",
            "scroll_view",
            json!({
                "size":[100,100], "scroll_content":"content", "scroll_view_port":"viewport",
                "scrollbar_track":"track", "scrollbar_box":"box", "scroll_box_and_track_panel":"bars"
            }),
            vec![
                viewport,
                node("bars", "panel", json!({"size":[4,100]}), vec![track]),
            ],
        );
        let boxed = Box::new(node("root", "panel", json!({"size":[100,100]}), vec![view]));
        let metrics = Metrics(Cell::new(0));
        let env = LayoutEnv {
            text: &metrics,
            textures: &metrics,
        };
        let mut cache = MeasureCache::default();
        let state = ViewState::default();
        let (_, first) =
            super::super::layout_cached(&boxed, [100.0, 100.0], &env, &state, &mut cache);
        assert_eq!(first.scrolls["/root/view"].bar_visible, Some(false));
        assert!(metrics.0.get() > 0);
        let moved = *boxed;
        metrics.0.set(0);
        let (_, next) =
            super::super::layout_cached(&moved, [100.0, 100.0], &env, &state, &mut cache);
        assert_eq!(first, next);
        assert_eq!(
            metrics.0.get(),
            0,
            "moving the root must not flush descendant measurements"
        );
    }
}
