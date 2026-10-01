//! Grid expansion: one `grid_item_template` instance per cell the grid holds,
//! the retained template, and the cell indices of grids listing their cells.

use std::collections::BTreeMap;
use std::sync::Arc;

use serde_json::Value;

use super::{Binder, Node, Scope, Src, with_index};
use crate::layout::GRID_TEMPLATE_KEY;
use crate::predicate::Scalar;
use crate::tree::{ControlRef, ResolvedControl};

impl Binder<'_> {
    /// One `grid_item_template` instance per cell the grid holds (columns × rows,
    /// or `maximum_grid_items` when rescaling), then the template itself, kept
    /// for layout to measure. A grid filling its extent holds one per item.
    pub(super) fn expand_grid(
        &mut self,
        src: &Src,
        own: &BTreeMap<String, Scalar>,
        template: &ControlRef,
        scope: &Scope,
    ) -> Vec<Node> {
        let control = src.get();
        let collection = control
            .properties
            .get("collection_name")
            .and_then(Value::as_str)
            .filter(|name| !name.is_empty());
        let key = collection.map(|name| self.collection_key(name, scope));
        let count = grid_capacity(src, own)
            .unwrap_or_else(|| key.as_ref().map_or(0, |key| self.data.collection_len(key)));
        let Some(resolved) = self.resolve_scoped(template, control, &BTreeMap::new()) else {
            self.diagnostics.push(format!(
                "{}: grid template {template} unresolved",
                control.name
            ));
            return Vec::new();
        };
        let mut cells: Vec<Node> = (0..count)
            .map(|index| {
                let child_scope = match (collection, &key) {
                    (Some(name), Some(key)) => scope.enter(name, key.clone(), index),
                    _ => scope.clone(),
                };
                let cell = with_index(Src::root(Arc::clone(&resolved)), index).patched(|patch| {
                    if let Some(name) = collection {
                        patch.properties.insert(
                            "collection_scope".to_owned(),
                            Value::String(name.to_owned()),
                        );
                    }
                });
                self.build(cell, &child_scope)
            })
            .collect();
        let template_node = Src::root(resolved).patched(|patch| {
            patch
                .properties
                .insert(GRID_TEMPLATE_KEY.to_owned(), Value::Bool(true));
        });
        cells.push(unbound(template_node));
        cells
    }
}

/// A node for a control and its subtree without binding: the grid template is
/// only measured, as the client's template binds no data.
fn unbound(src: Src) -> Node {
    let children = (0..src.get().children.len())
        .map(|index| unbound(src.child(index)))
        .collect();
    Node {
        src,
        own: BTreeMap::new(),
        children,
        deferred: None,
    }
}

/// A templated grid's cell count, or `None` when a filling grid's count waits
/// for layout: `maximum_grid_items` (a bound `#maximum_grid_items` winning, only
/// an integer counting) when rescaling, else columns × rows.
pub(super) fn grid_capacity(src: &Src, own: &BTreeMap<String, Scalar>) -> Option<usize> {
    let direction = |key: &str| {
        src.prop(key)
            .and_then(Value::as_str)
            .is_some_and(|value| value == "horizontal" || value == "vertical")
    };
    if direction("grid_rescaling_type") {
        let bound = own.get("#maximum_grid_items").map(|value| match value {
            Scalar::Num(number) if number.fract() == 0.0 && *number >= 0.0 => *number as usize,
            _ => 0,
        });
        let literal = || {
            src.prop("maximum_grid_items")
                .and_then(Value::as_f64)
                .filter(|max| max.fract() == 0.0 && *max >= 0.0)
                .map_or(0, |max| max as usize)
        };
        return Some(bound.unwrap_or_else(literal));
    }
    if direction("grid_fill_direction") {
        return None;
    }
    let dims = src.prop("grid_dimensions").and_then(Value::as_array);
    let int = |index: usize| {
        dims.and_then(|dims| dims.get(index)?.as_i64())
            .map_or(0, |value| value.max(0) as usize)
    };
    Some(int(0) * int(1))
}

/// The column count of a collection grid that lists its cells as children.
pub(super) fn static_grid_columns(control: &ResolvedControl) -> Option<u64> {
    if control.control_type.as_deref() != Some("grid")
        || !control.properties.contains_key("collection_name")
    {
        return None;
    }
    control
        .properties
        .get("grid_dimensions")?
        .as_array()?
        .first()?
        .as_u64()
        .filter(|columns| *columns > 0)
}

/// A static grid cell's collection index, row-major from its `grid_position`.
pub(super) fn grid_cell_index(control: &ResolvedControl, columns: u64) -> Option<usize> {
    if control.properties.contains_key("collection_index") {
        return None;
    }
    let position = control.properties.get("grid_position")?.as_array()?;
    let (column, row) = (position.first()?.as_u64()?, position.get(1)?.as_u64()?);
    usize::try_from(row * columns + column).ok()
}

/// The `grid_item_template` of a `grid`.
pub(super) fn grid_template(control: &ResolvedControl) -> Option<ControlRef> {
    if control.control_type.as_deref() != Some("grid") {
        return None;
    }
    let template = control
        .properties
        .get("grid_item_template")?
        .as_str()
        .filter(|template| !template.is_empty())?;
    let owner = control
        .base
        .as_ref()
        .map_or("", |base| base.namespace.as_str());
    Some(ControlRef::parse(template, owner))
}
