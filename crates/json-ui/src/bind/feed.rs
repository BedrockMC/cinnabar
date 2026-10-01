//! Named-factory feeds: the controls a screen controller creates through a
//! factory by name (chat lines, titles, the action bar), each resolved with its
//! property-bag `$vars` in the factory's scope, and the scope grids and
//! collection factories create their controls in.

use std::collections::BTreeMap;
use std::sync::Arc;

use serde_json::Value;

use super::{Binder, Node, Scope, Src};
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
    /// The collection cursor the created control's bindings read (a chat line
    /// reads its text from `chat_text_grid` at its own index).
    pub cursor: Option<(String, usize)>,
    /// A caller clock holding the creation time instead of `born`, so the
    /// control can restart its fade without the screen re-binding.
    pub clock: Option<String>,
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

    /// Read the creation time from the caller clock `name` at paint time.
    pub fn clocked(mut self, name: impl Into<String>) -> Self {
        self.clock = Some(name.into());
        self
    }

    /// Point the created control's collection bindings at `collection[index]`.
    pub fn at(mut self, collection: impl Into<String>, index: usize) -> Self {
        self.cursor = Some((collection.into(), index));
        self
    }

    /// Set a `#` property-bag value (keyed with the `#`).
    pub fn value(mut self, name: impl Into<String>, value: Scalar) -> Self {
        self.values.insert(name.into(), value);
        self
    }
}

impl<'a> Binder<'a> {
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
            let Some(resolved) = self.resolve_scoped(&reference, control, &item.vars) else {
                continue;
            };
            let instance = Src::root(resolved).patched(|patch| {
                patch.name.clone_from(&item.name);
                patch
                    .properties
                    .insert(crate::anim::BORN_KEY.to_owned(), Value::from(item.born));
                if let Some(clock) = &item.clock {
                    patch.properties.insert(
                        crate::anim::CLOCK_KEY.to_owned(),
                        Value::from(clock.clone()),
                    );
                }
            });
            // The item's property bag is readable throughout the created subtree.
            let mut item_scope = scope.clone();
            if let Some((collection, index)) = &item.cursor {
                std::sync::Arc::make_mut(&mut item_scope.cursor)
                    .indices
                    .insert(collection.clone(), *index as i64);
            }
            let mut values = (*item_scope.values).clone();
            values.extend(
                item.values
                    .iter()
                    .map(|(name, value)| (name.clone(), value.clone())),
            );
            item_scope.values = std::sync::Arc::new(values);
            nodes.push(self.build(instance, &item_scope));
        }
        nodes
    }

    /// Resolve `reference` in `scope`'s factory scope plus `extra` vars.
    pub(super) fn resolve_scoped(
        &mut self,
        reference: &ControlRef,
        scope: &ResolvedControl,
        extra: &BTreeMap<String, Value>,
    ) -> Option<Arc<ResolvedControl>> {
        let scope_key = scope
            .properties
            .get(crate::resolve::FACTORY_SCOPE_KEY)
            .and_then(Value::as_str)
            .unwrap_or("");
        if scope_key.is_empty() && extra.is_empty() {
            return self.resolve(reference);
        }
        let key = format!(
            "{scope_key}|{}",
            serde_json::to_string(extra).unwrap_or_default()
        );
        let cache_key = (reference.clone(), key);
        if let Some(resolved) = self.resolved_with.get(&cache_key) {
            return resolved.clone();
        }
        let vars = || {
            let mut vars = factory_scope(scope);
            vars.extend(
                extra
                    .iter()
                    .map(|(name, value)| (name.clone(), value.clone())),
            );
            vars
        };
        let resolved = self
            .lib
            .resolve_with(reference, &cache_key.1, &vars)
            .map(Arc::new);
        self.resolved_with.insert(cache_key, resolved.clone());
        resolved
    }
}

/// The `$vars` a factory or grid's created controls resolve with.
fn factory_scope(control: &ResolvedControl) -> BTreeMap<String, Value> {
    match control.properties.get(crate::resolve::FACTORY_SCOPE) {
        Some(Value::Object(vars)) => vars
            .iter()
            .map(|(name, value)| (name.clone(), value.clone()))
            .collect(),
        _ => BTreeMap::new(),
    }
}
