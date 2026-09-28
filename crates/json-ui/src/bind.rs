//! Data-binding layer: resolve `#bindings` against a screen data source and expand
//! `factory` collections into per-index control instances, producing a
//! [`ResolvedControl`] tree whose binding-driven properties (`text`, `texture`,
//! `visible`) are baked to literals and ready for [`crate::layout`]/[`crate::emit`].
//!
//! A data source is the screen's `global` values plus named `collection`s (one
//! per-index value map each). Binding evaluation runs in two passes: first every
//! control's own `global`/`collection` values are gathered top-down (so a factory's
//! collection index flows into the subtree it instantiates), then `view`
//! expressions — which may read a sibling or child control's bound values — resolve
//! and the properties bake. A `view` source that names no nearby control (e.g. an
//! ancestor the screen fed a property bag) reads the screen's global values. Every
//! bound value is also baked as a `#name` property so layout and emit can read
//! widget state (`#toggle_state`, `#slider_value`, `#enabled`). A `binding_name`
//! written as a parenthesised expression evaluates against the same scope.
//! Undecidable bindings are skipped, matching the lenient remote-data rule;
//! missing textures collapse to no sprite, not a broken one.

use std::collections::BTreeMap;

use serde_json::Value;

use crate::env::Env;
use crate::predicate::{self, Bindings, Scalar};
use crate::tree::{ControlRef, Factory, ResolvedControl};

/// One entry of a bound collection: the factory role that selects which control to
/// instantiate for this index, plus the `#name` values readable at it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CollectionItem {
    /// The `factory.control_ids` key for this index (e.g. `"button"`, `"toggle"`).
    /// `None` falls back to a single-target `control_name` or the sole control id.
    pub role: Option<String>,
    /// `#name` → value at this index, keyed with the leading `#`.
    pub values: BTreeMap<String, Scalar>,
}

impl CollectionItem {
    pub fn new(role: impl Into<String>) -> Self {
        Self {
            role: Some(role.into()),
            values: BTreeMap::new(),
        }
    }

    pub fn with(mut self, name: impl Into<String>, value: Scalar) -> Self {
        self.values.insert(name.into(), value);
        self
    }
}

/// The screen data source a form binds against: `global` values and named
/// collections.
#[derive(Clone, Debug, Default)]
pub struct DataSource {
    globals: BTreeMap<String, Scalar>,
    collections: BTreeMap<String, Vec<CollectionItem>>,
}

impl DataSource {
    pub fn new() -> Self {
        Self::default()
    }

    /// Set a `global` binding value, keyed with its leading `#`.
    pub fn set_global(&mut self, name: impl Into<String>, value: Scalar) {
        self.globals.insert(name.into(), value);
    }

    /// Replace a named collection's per-index items.
    pub fn set_collection(&mut self, name: impl Into<String>, items: Vec<CollectionItem>) {
        self.collections.insert(name.into(), items);
    }

    fn collection_len(&self, name: &str) -> usize {
        self.collections.get(name).map_or(0, Vec::len)
    }
}

/// Resolves a factory `control_ids`/`control_name` reference to a fresh control
/// tree. The renderer backs this with the catalog; tests can stub it.
pub trait ControlLibrary {
    fn resolve(&self, reference: &ControlRef) -> Option<ResolvedControl>;
}

/// A [`ControlLibrary`] with no definitions; binding a tree that needs no factory
/// expansion (e.g. binding unit tests) can use it.
pub struct EmptyLibrary;

impl ControlLibrary for EmptyLibrary {
    fn resolve(&self, _reference: &ControlRef) -> Option<ResolvedControl> {
        None
    }
}

/// Bind `root` against `data`, expanding factory collections via `lib`, and return
/// the baked tree.
pub fn bind(
    root: &ResolvedControl,
    data: &DataSource,
    lib: &dyn ControlLibrary,
) -> ResolvedControl {
    let mut binder = Binder {
        data,
        lib,
        env: Env::new(),
        diagnostics: Vec::new(),
    };
    let node = binder.build(root, &Scope::default());
    binder.bake(&node, &[])
}

/// The active collection cursors: `collection_name` → the current index its
/// `collection` bindings read from. A factory sets one when it instantiates a child.
#[derive(Clone, Debug, Default)]
struct Scope {
    indices: BTreeMap<String, usize>,
}

/// A control plus the own values gathered for it (pass one), before `view`
/// resolution and property baking (pass two).
struct Node {
    control: ResolvedControl,
    own: BTreeMap<String, Scalar>,
    children: Vec<Node>,
}

struct Binder<'a> {
    data: &'a DataSource,
    lib: &'a dyn ControlLibrary,
    env: Env,
    diagnostics: Vec<String>,
}

impl Binder<'_> {
    /// Pass one: gather own `global`/`collection` values and expand factories.
    fn build(&mut self, control: &ResolvedControl, scope: &Scope) -> Node {
        let own = self.gather_own(control, scope);
        let children = if is_collection_factory(control) {
            self.expand_factory(control, scope)
        } else if let Some(template) = grid_template(control) {
            self.expand_grid(control, &template, scope)
        } else {
            control
                .children
                .iter()
                .map(|child| self.build(child, scope))
                .collect()
        };
        Node {
            control: without_children(control),
            own,
            children,
        }
    }

    fn gather_own(&self, control: &ResolvedControl, scope: &Scope) -> BTreeMap<String, Scalar> {
        let mut own = property_bag(control);
        for binding in bindings_of(control) {
            let Some(binding) = binding.as_object() else {
                continue;
            };
            match binding.get("binding_type").and_then(Value::as_str) {
                Some("collection") => {
                    let Some(collection) = binding
                        .get("binding_collection_name")
                        .and_then(Value::as_str)
                    else {
                        continue;
                    };
                    let Some(&index) = scope.indices.get(collection) else {
                        continue;
                    };
                    let Some(source) = binding.get("binding_name").and_then(Value::as_str) else {
                        continue;
                    };
                    let Some(item) = self
                        .data
                        .collections
                        .get(collection)
                        .and_then(|items| items.get(index))
                    else {
                        continue;
                    };
                    if let Some(value) = lookup(source, &item.values, &own, &self.env) {
                        own.insert(target_name(binding, source), value);
                    }
                }
                // Establishes the subtree cursor; the factory already set it, so this
                // is a recognised confirmation rather than a change.
                Some("collection_details") => {}
                // `view` reads other controls' values; deferred to pass two.
                Some("view") => {}
                // Explicitly inert, or an unrecognised type: skip leniently.
                Some("none") => {}
                Some("global") | None => {
                    let Some(source) = binding.get("binding_name").and_then(Value::as_str) else {
                        continue;
                    };
                    if let Some(value) = lookup(source, &self.data.globals, &own, &self.env) {
                        own.insert(target_name(binding, source), value);
                    }
                }
                Some(_) => {}
            }
        }
        own
    }

    fn expand_factory(&mut self, control: &ResolvedControl, scope: &Scope) -> Vec<Node> {
        let Some(factory) = &control.factory else {
            return Vec::new();
        };
        let Some(collection) = control
            .properties
            .get("collection_name")
            .and_then(Value::as_str)
        else {
            return Vec::new();
        };
        let count = self.data.collection_len(collection);
        let mut nodes = Vec::with_capacity(count);
        for index in 0..count {
            let role = self.data.collections[collection][index].role.as_deref();
            let Some(reference) = select_control(factory, role) else {
                self.diagnostics.push(format!(
                    "{}: factory has no control for role {role:?}",
                    control.name
                ));
                continue;
            };
            let Some(resolved) = self.lib.resolve(reference) else {
                self.diagnostics.push(format!(
                    "{}: factory control {reference} unresolved",
                    control.name
                ));
                continue;
            };
            let mut child_scope = scope.clone();
            child_scope.indices.insert(collection.to_owned(), index);
            nodes.push(self.build(&with_index(resolved, index), &child_scope));
        }
        nodes
    }

    /// One `grid_item_template` instance per collection item, capped by
    /// `maximum_grid_items` when set.
    fn expand_grid(
        &mut self,
        control: &ResolvedControl,
        template: &ControlRef,
        scope: &Scope,
    ) -> Vec<Node> {
        let Some(collection) = control
            .properties
            .get("collection_name")
            .and_then(Value::as_str)
        else {
            return Vec::new();
        };
        let cap = control
            .properties
            .get("maximum_grid_items")
            .and_then(Value::as_u64)
            .map_or(usize::MAX, |cap| cap as usize);
        // A fixed grid always has `columns * rows` cells; a rescaling one follows
        // its collection.
        let dimensions = control
            .properties
            .get("grid_dimensions")
            .and_then(Value::as_array)
            .and_then(|dims| Some(dims.first()?.as_u64()? * dims.get(1)?.as_u64()?));
        let count = dimensions
            .map_or_else(
                || self.data.collection_len(collection),
                |cells| cells as usize,
            )
            .min(cap);
        let Some(resolved) = self.lib.resolve(template) else {
            self.diagnostics.push(format!(
                "{}: grid template {template} unresolved",
                control.name
            ));
            return Vec::new();
        };
        (0..count)
            .map(|index| {
                let mut child_scope = scope.clone();
                child_scope.indices.insert(collection.to_owned(), index);
                let mut cell = with_index(resolved.clone(), index);
                cell.properties.insert(
                    "collection_scope".to_owned(),
                    Value::String(collection.to_owned()),
                );
                self.build(&cell, &child_scope)
            })
            .collect()
    }

    /// Pass two: resolve `view` bindings against sibling/child values, then bake
    /// binding-driven properties into literals.
    fn bake(&self, node: &Node, siblings: &[Node]) -> ResolvedControl {
        let mut own = node.own.clone();
        self.resolve_views(node, siblings, &mut own);
        let properties = bake_properties(&node.control.properties, &own);
        let children = node
            .children
            .iter()
            .map(|child| self.bake(child, &node.children))
            .collect();
        ResolvedControl {
            properties,
            children,
            ..node.control.clone()
        }
    }

    fn resolve_views(&self, node: &Node, siblings: &[Node], own: &mut BTreeMap<String, Scalar>) {
        for binding in bindings_of(&node.control) {
            let Some(binding) = binding.as_object() else {
                continue;
            };
            if binding.get("binding_type").and_then(Value::as_str) != Some("view") {
                continue;
            }
            let (Some(expression), Some(target)) = (
                binding.get("source_property_name").and_then(Value::as_str),
                binding.get("target_property_name").and_then(Value::as_str),
            ) else {
                continue;
            };
            let source = binding
                .get("source_control_name")
                .and_then(Value::as_str)
                .filter(|name| !name.is_empty());
            let snapshot;
            let scope_values = match source {
                Some(name) => find_own(name, &node.children)
                    .or_else(|| find_own(name, siblings))
                    .unwrap_or(&self.data.globals),
                None => {
                    snapshot = own.clone();
                    &snapshot
                }
            };
            if let Some(value) =
                predicate::eval_scalar(expression, &self.env, &MapBindings(scope_values))
            {
                own.insert(target.to_owned(), value);
            }
        }
    }
}

/// Read a control's `bindings` array; a non-array (or absent) yields nothing.
fn bindings_of(control: &ResolvedControl) -> &[Value] {
    match control.properties.get("bindings") {
        Some(Value::Array(items)) => items,
        _ => &[],
    }
}

fn target_name(binding: &serde_json::Map<String, Value>, source: &str) -> String {
    binding
        .get("binding_name_override")
        .and_then(Value::as_str)
        .unwrap_or(source)
        .to_owned()
}

fn is_collection_factory(control: &ResolvedControl) -> bool {
    control.factory.is_some() && control.properties.contains_key("collection_name")
}

fn select_control<'a>(factory: &'a Factory, role: Option<&str>) -> Option<&'a ControlRef> {
    if let Some(role) = role
        && let Some(reference) = factory.control_ids.get(role)
    {
        return Some(reference);
    }
    factory
        .control_name
        .as_ref()
        .or_else(|| factory.control_ids.values().next())
}

fn without_children(control: &ResolvedControl) -> ResolvedControl {
    ResolvedControl {
        children: Vec::new(),
        ..control.clone()
    }
}

fn find_own<'a>(name: &str, nodes: &'a [Node]) -> Option<&'a BTreeMap<String, Scalar>> {
    for node in nodes {
        if node.control.name == name {
            return Some(&node.own);
        }
    }
    for node in nodes {
        if let Some(found) = find_own(name, &node.children) {
            return Some(found);
        }
    }
    None
}

/// Replace `#`-referencing property values and binding-target properties with their
/// bound literals. Empty textures collapse to no property so no sprite is emitted;
/// an unbound `text` becomes empty rather than the literal `#name`.
fn bake_properties(
    properties: &BTreeMap<String, Value>,
    own: &BTreeMap<String, Scalar>,
) -> BTreeMap<String, Value> {
    let mut out = BTreeMap::new();
    for (key, value) in properties {
        match value {
            Value::String(reference) if reference.starts_with('#') => match own.get(reference) {
                Some(scalar) => {
                    out.insert(key.clone(), scalar_to_value(scalar));
                }
                None if key == "text" => {
                    out.insert(key.clone(), Value::String(String::new()));
                }
                None => {}
            },
            _ => {
                out.insert(key.clone(), value.clone());
            }
        }
    }
    for (name, value) in own {
        if name.starts_with('#') {
            out.insert(name.clone(), scalar_to_value(value));
        }
    }
    if let Some(visible) = own.get("#visible").and_then(Scalar::as_bool) {
        out.insert("visible".to_owned(), Value::Bool(visible));
    }
    if let Some(ratio) = own.get("#clip_ratio").and_then(scalar_number) {
        out.insert(
            "clip_ratio".to_owned(),
            scalar_to_value(&Scalar::Num(ratio)),
        );
    }
    if let Some(texture) =
        nonempty_text(own.get("#texture")).or(nonempty_text(own.get("#texture_file_system")))
    {
        out.insert("texture".to_owned(), Value::String(texture));
    }
    out
}

fn scalar_number(scalar: &Scalar) -> Option<f64> {
    match scalar {
        Scalar::Num(number) => Some(*number),
        Scalar::Text(text) => text.parse().ok(),
        Scalar::Bool(_) => None,
    }
}

/// Resolve a `binding_name` against `values`: a plain `#name` looks up directly, a
/// parenthesised expression evaluates with `values` (then the control's own
/// property-bag values) as its binding scope.
fn lookup(
    source: &str,
    values: &BTreeMap<String, Scalar>,
    own: &BTreeMap<String, Scalar>,
    env: &Env,
) -> Option<Scalar> {
    if source.starts_with('(') {
        return predicate::eval_scalar(source, env, &LayeredBindings(values, own));
    }
    values.get(source).cloned()
}

struct LayeredBindings<'a>(&'a BTreeMap<String, Scalar>, &'a BTreeMap<String, Scalar>);

impl Bindings for LayeredBindings<'_> {
    fn get(&self, name: &str) -> Option<Scalar> {
        self.0.get(name).or_else(|| self.1.get(name)).cloned()
    }
}

/// A control's `property_bag`: initial `#` values that bindings then override.
fn property_bag(control: &ResolvedControl) -> BTreeMap<String, Scalar> {
    let mut own = BTreeMap::new();
    if let Some(Value::Object(bag)) = control.properties.get("property_bag") {
        for (name, value) in bag {
            let scalar = match value {
                Value::Bool(flag) => Scalar::Bool(*flag),
                Value::Number(number) => match number.as_f64() {
                    Some(number) => Scalar::Num(number),
                    None => continue,
                },
                Value::String(text) => Scalar::Text(text.clone()),
                _ => continue,
            };
            own.insert(name.clone(), scalar);
        }
    }
    own
}

/// A factory/grid instance records its collection index for keys and events.
fn with_index(mut control: ResolvedControl, index: usize) -> ResolvedControl {
    control
        .properties
        .insert("collection_index".to_owned(), Value::from(index as u64));
    control
}

/// The `grid_item_template` of a collection-bound `grid`.
fn grid_template(control: &ResolvedControl) -> Option<ControlRef> {
    if control.control_type.as_deref() != Some("grid") {
        return None;
    }
    let template = control.properties.get("grid_item_template")?.as_str()?;
    control.properties.get("collection_name")?;
    let owner = control
        .base
        .as_ref()
        .map_or("", |base| base.namespace.as_str());
    Some(ControlRef::parse(template, owner))
}

fn nonempty_text(scalar: Option<&Scalar>) -> Option<String> {
    match scalar {
        Some(Scalar::Text(text)) if !text.is_empty() => Some(text.clone()),
        _ => None,
    }
}

fn scalar_to_value(scalar: &Scalar) -> Value {
    match scalar {
        Scalar::Bool(value) => Value::Bool(*value),
        Scalar::Text(text) => Value::String(text.clone()),
        Scalar::Num(number) => serde_json::Number::from_f64(*number)
            .map(Value::Number)
            .unwrap_or(Value::Null),
    }
}

struct MapBindings<'a>(&'a BTreeMap<String, Scalar>);

impl Bindings for MapBindings<'_> {
    fn get(&self, name: &str) -> Option<Scalar> {
        self.0.get(name).cloned()
    }
}
