//! Named-factory feeds: the controls a screen controller creates through a
//! factory by name (chat lines, titles, the action bar), each resolved with its
//! property-bag `$vars` in the factory's scope, and the scope grids and
//! collection factories create their controls in.

use std::collections::BTreeMap;
use std::sync::Arc;

use serde_json::Value;

use super::{Binder, Node, Scope, Src, with_index};
use crate::predicate::Scalar;
use crate::tree::{ControlRef, Factory, ResolvedControl};

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
    /// The `[columns, rows]` the screen answers a grid's `grid_dimension_binding` with.
    pub(super) fn bound_dimensions(&self, control: &ResolvedControl) -> Option<[u32; 2]> {
        let name = control
            .properties
            .get("grid_dimension_binding")
            .and_then(Value::as_str)?;
        self.data.grid_dimensions.get(name).copied()
    }

    /// The items a screen fed to this control's named factory.
    pub(super) fn feed(&self, control: &ResolvedControl) -> Option<&'a [FactoryItem]> {
        let name = control.factory.as_ref()?.name.as_deref()?;
        self.data.factories.get(name).map(Vec::as_slice)
    }

    /// One control per fed item, oldest first: a `control_name` template as
    /// declared, else the item's `control_ids` entry with its own and the
    /// captured variables; an id the factory lacks creates nothing.
    pub(super) fn expand_feed(
        &mut self,
        control: &ResolvedControl,
        items: &[FactoryItem],
        scope: &Scope,
    ) -> Vec<Node> {
        let Some(factory) = control.factory.clone() else {
            return Vec::new();
        };
        let mut nodes = Vec::new();
        for item in items {
            let (reference, vars) = match &factory.control_name {
                Some(template) => (template.clone(), BTreeMap::new()),
                None => match factory.control_ids.get(&item.control_id) {
                    Some(reference) => (reference.clone(), factory.creation_vars(&item.vars)),
                    None => continue,
                },
            };
            let Some(resolved) = self.resolve_scoped(&reference, control, &vars) else {
                continue;
            };
            let name = item
                .name
                .clone()
                .or_else(|| factory.instance_names.get(&item.control_id).cloned());
            let instance = Src::root(resolved).patched(|patch| {
                patch.name.clone_from(&name);
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
                item_scope.indices.insert(collection.clone(), *index);
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

    pub(super) fn expand_factory(
        &mut self,
        control: &ResolvedControl,
        own: &BTreeMap<String, Scalar>,
        scope: &Scope,
    ) -> Vec<Node> {
        let Some(factory) = &control.factory else {
            return Vec::new();
        };
        let Some(collection) = collection_name(control) else {
            return Vec::new();
        };
        let key = self.collection_key(collection, scope);
        let roles: Vec<Option<String>> = match self.data.collections.get(&key) {
            Some(items) => items.iter().map(|item| item.role.clone()).collect(),
            None => unsupplied_roles(control, factory, own),
        };
        let mut nodes = Vec::with_capacity(roles.len());
        for (index, role) in roles.iter().enumerate() {
            let role = role.as_deref();
            let Some(reference) = select_control(factory, role) else {
                self.diagnostics.push(format!(
                    "{}: factory has no control for role {role:?}",
                    control.name
                ));
                continue;
            };
            let reference = reference.clone();
            let vars = match factory.control_name {
                Some(_) => BTreeMap::new(),
                None => factory.creation_vars(&BTreeMap::new()),
            };
            let Some(resolved) = self.resolve_scoped(&reference, control, &vars) else {
                self.diagnostics.push(format!(
                    "{}: factory control {reference} unresolved",
                    control.name
                ));
                continue;
            };
            let child_scope = scope.enter(collection, key.clone(), index);
            nodes.push(self.build(with_index(Src::root(resolved), index), &child_scope));
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

/// Most instances a factory makes for a collection the screen does not supply.
const MAX_UNSUPPLIED_ITEMS: usize = 64;

/// Roles for a collection the screen does not supply, from `#collection_length`:
/// an array of control ids makes one instance per id; a number makes that many
/// only for a `control_name` template, since an id-mapped factory needs ids.
fn unsupplied_roles(
    control: &ResolvedControl,
    factory: &Factory,
    own: &BTreeMap<String, Scalar>,
) -> Vec<Option<String>> {
    let ids = control
        .properties
        .get("property_bag")
        .and_then(|bag| bag.get("#collection_length"))
        .and_then(Value::as_array);
    if let Some(ids) = ids {
        return ids
            .iter()
            .take(MAX_UNSUPPLIED_ITEMS)
            .map(|id| id.as_str().map(str::to_owned))
            .collect();
    }
    match own.get("#collection_length") {
        Some(Scalar::Num(length)) if *length > 0.0 && factory.control_name.is_some() => {
            vec![None; (*length as usize).min(MAX_UNSUPPLIED_ITEMS)]
        }
        _ => Vec::new(),
    }
}

/// A control's `collection_name`; an empty one declares no collection.
pub(super) fn collection_name(control: &ResolvedControl) -> Option<&str> {
    let name = control.properties.get("collection_name")?.as_str()?;
    (!name.is_empty()).then_some(name)
}

pub(super) fn is_collection_factory(control: &ResolvedControl) -> bool {
    control.factory.is_some() && collection_name(control).is_some()
}

/// The control a collection item creates: the template, else its role's entry
/// (an item without a role takes the first); a role the factory lacks creates nothing.
fn select_control<'a>(factory: &'a Factory, role: Option<&str>) -> Option<&'a ControlRef> {
    if let Some(template) = &factory.control_name {
        return Some(template);
    }
    match role {
        Some(role) => factory.control_ids.get(role),
        None => factory.control_ids.values().next(),
    }
}
