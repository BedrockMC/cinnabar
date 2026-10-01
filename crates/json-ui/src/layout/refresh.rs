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
