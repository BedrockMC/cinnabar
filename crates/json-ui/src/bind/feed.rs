//! Named-factory feeds: the controls a screen controller creates through a
//! factory by name (chat lines, titles, the action bar), each resolved with its
//! property-bag `$vars` in the factory's scope, and the scope grids and
//! collection factories create their controls in.

use std::collections::BTreeMap;

use serde_json::Value;

use super::{Binder, Node, Scope};
use crate::predicate::Scalar;
use crate::tree::{ControlRef, ResolvedControl};

/// One control a screen controller asked a named factory to create: the
/// `control_ids` entry, the instance name, the `$vars` it resolves with, the `#`
/// values of its property bag, and when it was created (seconds, the caller's
/// animation clock).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FactoryItem {
    pub control_id: String,
    pub name: Option<String>,
    pub vars: BTreeMap<String, Value>,
    pub values: BTreeMap<String, Scalar>,
    pub born: f64,
}

impl FactoryItem {
    pub fn new(control_id: impl Into<String>, born: f64) -> Self {
        Self {
            control_id: control_id.into(),
            born,
            ..Self::default()
        }
    }

    pub fn named(mut self, name: impl Into<String>) -> Self {
        self.name = Some(name.into());
        self
    }

    /// Set a `$var` (keyed without the `$`).
    pub fn var(mut self, name: impl Into<String>, value: Value) -> Self {
        self.vars.insert(name.into(), value);
        self
    }

    /// Set a `#` property-bag value (keyed with the `#`).
    pub fn value(mut self, name: impl Into<String>, value: Scalar) -> Self {
        self.values.insert(name.into(), value);
        self
    }
}

impl<'a> Binder<'a> {
    /// A grid whose `grid_dimension_binding` the screen answers, with those
    /// dimensions set; `None` leaves the control as authored.
    pub(super) fn bound_dimensions(&self, control: &ResolvedControl) -> Option<ResolvedControl> {
        let name = control
            .properties
            .get("grid_dimension_binding")
            .and_then(Value::as_str)?;
        let [columns, rows] = *self.data.grid_dimensions.get(name)?;
        let mut control = control.clone();
        control.properties.insert(
            "grid_dimensions".to_owned(),
            Value::from(vec![columns, rows]),
        );
        Some(control)
    }

    /// The items a screen fed to this control's named factory.
    pub(super) fn feed(&self, control: &ResolvedControl) -> Option<&'a [FactoryItem]> {
        let name = control.factory.as_ref()?.name.as_deref()?;
        self.data.factories.get(name).map(Vec::as_slice)
    }

    /// One control per fed item, newest `max_children_size` kept.
    pub(super) fn expand_feed(
        &mut self,
        control: &ResolvedControl,
        items: &[FactoryItem],
        scope: &Scope,
    ) -> Vec<Node> {
        let Some(factory) = control.factory.clone() else {
            return Vec::new();
        };
        let skip = factory
            .max_children_size
            .map_or(0, |max| items.len().saturating_sub(max));
        let mut nodes = Vec::new();
        for item in &items[skip..] {
            let Some(reference) = factory
                .control_ids
                .get(&item.control_id)
                .or(factory.control_name.as_ref())
                .cloned()
            else {
                continue;
            };
            let mut vars = factory_scope(control);
            vars.extend(
                item.vars
                    .iter()
                    .map(|(name, value)| (name.clone(), value.clone())),
            );
            let Some(mut instance) = self.resolve_with(&reference, &vars) else {
                continue;
            };
            if let Some(name) = &item.name {
                instance.name = name.clone();
            }
            instance
                .properties
                .insert(crate::anim::BORN_KEY.to_owned(), Value::from(item.born));
            // The item's property bag is readable throughout the created subtree.
            let mut item_scope = scope.clone();
            let mut values = (*item_scope.values).clone();
            values.extend(
                item.values
                    .iter()
                    .map(|(name, value)| (name.clone(), value.clone())),
            );
            item_scope.values = std::sync::Arc::new(values);
            nodes.push(self.build(&instance, &item_scope));
        }
        nodes
    }

    pub(super) fn resolve_with(
        &mut self,
        reference: &ControlRef,
        vars: &BTreeMap<String, Value>,
    ) -> Option<ResolvedControl> {
        if vars.is_empty() {
            return self.resolve(reference);
        }
        let key = (
            reference.clone(),
            serde_json::to_string(vars).unwrap_or_default(),
        );
        if let Some(resolved) = self.resolved_with.get(&key) {
            return resolved.clone();
        }
        let resolved = self.lib.resolve_with(reference, vars);
        self.resolved_with.insert(key, resolved.clone());
        resolved
    }
}

/// The `$vars` a factory or grid's created controls resolve with.
pub(super) fn factory_scope(control: &ResolvedControl) -> BTreeMap<String, Value> {
    match control.properties.get(crate::resolve::FACTORY_SCOPE) {
        Some(Value::Object(vars)) => vars
            .iter()
            .map(|(name, value)| (name.clone(), value.clone()))
            .collect(),
        _ => BTreeMap::new(),
    }
}
