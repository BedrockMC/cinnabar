//! Data-binding layer: resolve `#bindings` against a screen data source and expand
//! `factory` collections into per-index control instances, producing a
//! [`ResolvedControl`] tree whose binding-driven properties (`text`, `texture`,
//! `visible`) are baked to literals and ready for [`crate::layout`]/[`crate::emit`].
//!
//! A data source is the screen's `global` values plus named `collection`s (one
//! per-index value map each). Binding evaluation runs in two passes: first every
//! control's own `global`/`collection` values are gathered top-down (so a factory's
//! or a literal `collection_index`'s item flows into its subtree), then `view`
//! expressions settle across the tree, finding a named source the way the vanilla
//! client does, and the properties bake. A hidden subtree builds only once a view
//! shows it. A source-less `view` reads its own values, then its ancestors', then
//! the screen's globals. Every
//! bound value is also baked as a `#name` property so layout and emit can read
//! widget state (`#toggle_state`, `#slider_value`, `#enabled`). A `binding_name`
//! written as a parenthesised expression evaluates against the same scope.
//! Undecidable bindings are skipped, matching the lenient remote-data rule;
//! missing textures collapse to no sprite, not a broken one.

use std::collections::BTreeMap;
use std::sync::Arc;

use serde_json::Value;

use crate::env::Env;
use crate::predicate::{self, Bindings, Scalar};
use crate::tree::{ControlRef, Factory, ResolvedControl};

mod data;
mod feed;
mod grid;
mod source;

pub use data::{CollectionItem, DataSource, scoped_key};
pub use feed::FactoryItem;
use grid::{grid_capacity, grid_cell_index, grid_template, static_grid_columns};
use source::Src;

/// Resolves a factory `control_ids`/`control_name` reference to a fresh control
/// tree. The renderer backs this with the catalog; tests can stub it.
pub trait ControlLibrary {
    fn resolve(&self, reference: &ControlRef) -> Option<ResolvedControl>;

    /// Resolve with extra `$vars` in scope, as a factory creates a control.
    /// `key` identifies the vars (equal keys, equal vars), so a caching library
    /// can answer without building them.
    fn resolve_with(
        &self,
        reference: &ControlRef,
        key: &str,
        vars: &dyn Fn() -> BTreeMap<String, Value>,
    ) -> Option<ResolvedControl> {
        let _ = (key, vars);
        self.resolve(reference)
    }
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
    bind_shared(&Arc::new(root.clone()), data, lib)
}

/// [`bind`] over a shared tree the caller keeps across binds, which the binder
/// reads in place rather than copying.
pub fn bind_shared(
    root: &Arc<ResolvedControl>,
    data: &DataSource,
    lib: &dyn ControlLibrary,
) -> ResolvedControl {
    bind_reporting(root, data, lib).0
}

/// [`bind_shared`] plus what the binder skipped: factory roles with no control
/// and factory or grid templates that did not resolve.
pub fn bind_reporting(
    root: &Arc<ResolvedControl>,
    data: &DataSource,
    lib: &dyn ControlLibrary,
) -> (ResolvedControl, Vec<String>) {
    let mut binder = Binder {
        data,
        lib,
        env: Env::new(),
        diagnostics: Vec::new(),
        resolved: BTreeMap::new(),
        resolved_with: BTreeMap::new(),
    };
    let mut node = binder.build(Src::root(Arc::clone(root)), &Scope::default());
    for _ in 0..EXPANSION_ROUNDS {
        binder.settle_views(&mut node);
        if !binder.expand_deferred(&mut node, true) {
            break;
        }
    }
    let baked = binder.bake(&node);
    (baked, binder.diagnostics)
}

/// Rounds of settling views then building the subtrees they revealed.
const EXPANSION_ROUNDS: usize = 8;
/// The collection a `collection_details` binding names, for hit regions.
pub(crate) const COLLECTION_NAME_KEY: &str = "#collection_name";

/// Whether a control's own values or literal `visible` hide it.
fn hidden(control: &ResolvedControl, own: &BTreeMap<String, Scalar>) -> bool {
    match own.get("#visible").and_then(Scalar::as_bool) {
        Some(visible) => !visible,
        None => control.properties.get("visible") == Some(&Value::Bool(false)),
    }
}

/// The active collection cursors: `collection_name` → the current index its
/// `collection` bindings read from. A factory sets one when it instantiates a child.
#[derive(Clone, Debug, Default)]
struct Scope {
    indices: BTreeMap<String, usize>,
    /// The nearest enclosing `collection_name`, which a child's literal
    /// `collection_index` points into.
    panel: Option<String>,
    /// A factory item's property bag, the base every control under it binds on.
    values: std::sync::Arc<BTreeMap<String, Scalar>>,
    /// The data key each active cursor's collection resolved to.
    keys: BTreeMap<String, String>,
    /// Entered collections, outermost first: `(data key, index)`.
    path: Vec<(String, usize)>,
}

impl Scope {
    /// The scope inside item `index` of `name`, whose list lives at `key`.
    fn enter(&self, name: &str, key: String, index: usize) -> Scope {
        let mut inner = self.clone();
        inner.indices.insert(name.to_owned(), index);
        inner.keys.insert(name.to_owned(), key.clone());
        inner.path.push((key, index));
        inner
    }
}

/// A control plus the own values gathered for it (pass one), before `view`
/// resolution and property baking (pass two).
struct Node {
    src: Src,
    own: BTreeMap<String, Scalar>,
    children: Vec<Node>,
    /// A hidden control's scope, kept to build its subtree once shown.
    deferred: Option<Scope>,
}

struct Binder<'a> {
    data: &'a DataSource,
    lib: &'a dyn ControlLibrary,
    env: Env,
    diagnostics: Vec<String>,
    /// Library resolutions memoized per reference, shared by their instances.
    resolved: BTreeMap<ControlRef, Option<Arc<ResolvedControl>>>,
    /// Factory-item resolutions, keyed by reference and serialized `$vars`.
    resolved_with: BTreeMap<(ControlRef, String), Option<Arc<ResolvedControl>>>,
}

impl<'a> Binder<'a> {
    /// Pass one: gather own `global`/`collection` values and expand factories.
    fn build(&mut self, src: Src, scope: &Scope) -> Node {
        let src = match self.bound_dimensions(src.get()) {
            Some([columns, rows]) => src.patched(|patch| {
                patch.properties.insert(
                    "grid_dimensions".to_owned(),
                    Value::from(vec![columns, rows]),
                );
            }),
            None => src,
        };
        let control = src.get();
        let mut scope = scope.clone();
        if let (Some(panel), Some(index)) = (
            scope.panel.clone(),
            src.prop("collection_index").and_then(Value::as_u64),
        ) {
            scope.indices.insert(panel, index as usize);
        }
        let mut own = self.gather_own(control, &scope);
        if control.control_type.as_deref() == Some("grid")
            && let Some(capacity) = grid_capacity(&src, &own)
        {
            own.insert("#grid_number_size".to_owned(), Scalar::Num(capacity as f64));
        }
        if let Some(name) = control
            .properties
            .get("collection_name")
            .and_then(Value::as_str)
        {
            scope.panel = Some(name.to_owned());
        }
        // A hidden control's subtree builds only once views show it, so a
        // pack's many title-selected layouts cost only the one on screen.
        if hidden(control, &own) {
            return Node {
                src,
                own,
                children: Vec::new(),
                deferred: Some(scope),
            };
        }
        let children = self.children_of(&src, &own, &scope);
        Node {
            src,
            own,
            children,
            deferred: None,
        }
    }

    fn children_of(
        &mut self,
        src: &Src,
        own: &BTreeMap<String, Scalar>,
        scope: &Scope,
    ) -> Vec<Node> {
        let control = src.get();
        if is_collection_factory(control) {
            self.expand_factory(control, own, scope)
        } else if let Some(reference) = self.screen_factory(control) {
            self.resolve(&reference)
                .map(|resolved| vec![self.build(Src::root(resolved), scope)])
                .unwrap_or_default()
        } else if let Some(items) = self.feed(control) {
            self.expand_feed(control, items, scope)
        } else if let Some(template) = grid_template(control) {
            self.expand_grid(src, own, &template, scope)
        } else {
            let columns = static_grid_columns(control);
            (0..control.children.len())
                .map(|index| {
                    let child = src.child(index);
                    match columns.and_then(|columns| grid_cell_index(child.get(), columns)) {
                        Some(at) => self.build(with_index(child, at), scope),
                        None => self.build(child, scope),
                    }
                })
                .collect()
        }
    }

    fn resolve(&mut self, reference: &ControlRef) -> Option<Arc<ResolvedControl>> {
        if let Some(resolved) = self.resolved.get(reference) {
            return resolved.clone();
        }
        let resolved = self.lib.resolve(reference).map(Arc::new);
        self.resolved.insert(reference.clone(), resolved.clone());
        resolved
    }

    /// Build deferred subtrees that are now shown under shown ancestors; `true`
    /// when any was built.
    fn expand_deferred(&mut self, node: &mut Node, parent_visible: bool) -> bool {
        let visible = parent_visible && !hidden(node.src.get(), &node.own);
        if !visible {
            return false;
        }
        let mut expanded = false;
        if let Some(scope) = node.deferred.take() {
            let src = node.src.clone();
            node.children = self.children_of(&src, &node.own, &scope);
            expanded = true;
        }
        for child in &mut node.children {
            expanded |= self.expand_deferred(child, true);
        }
        expanded
    }

    /// The control a collection-less factory instantiates for the screen's id.
    fn screen_factory(&self, control: &ResolvedControl) -> Option<ControlRef> {
        let factory = control.factory.as_ref()?;
        factory
            .control_ids
            .get(self.data.factory_id.as_deref()?)
            .cloned()
    }

    fn gather_own(&self, control: &ResolvedControl, scope: &Scope) -> BTreeMap<String, Scalar> {
        let mut own = (*scope.values).clone();
        own.extend(property_bag(control));
        for binding in bindings_of(control) {
            let Some(binding) = binding.as_object() else {
                continue;
            };
            match binding.get("binding_type").and_then(Value::as_str) {
                Some("collection") => {
                    let Some(source) = binding.get("binding_name").and_then(Value::as_str) else {
                        continue;
                    };
                    let item = binding
                        .get("binding_collection_name")
                        .and_then(Value::as_str)
                        .and_then(|collection| {
                            // Outside its grid a control reads the collection's first item.
                            let index = scope.indices.get(collection).copied().unwrap_or(0);
                            let key = scope
                                .keys
                                .get(collection)
                                .map_or(collection, String::as_str);
                            self.data.collections.get(key)?.get(index)
                        });
                    // The controller answers a collection's size itself.
                    let total = (source == "#collection_total_items")
                        .then(|| binding.get("binding_collection_name")?.as_str())
                        .flatten()
                        .and_then(|collection| {
                            let key = scope
                                .keys
                                .get(collection)
                                .map_or(collection, String::as_str);
                            self.data.collections.get(key)
                        })
                        .map(|items| Scalar::Num(items.len() as f64));
                    let value = total.or_else(|| {
                        item.and_then(|item| {
                            lookup(source, &item.values, &own, self.data.strict, &self.env)
                        })
                    });
                    let target = target_name(binding, source);
                    match value {
                        Some(value) => {
                            own.insert(target, value);
                        }
                        // A controller answers a collection flag it lacks (an item
                        // lock, a collection it does not own) with `false`.
                        None if self.data.strict && target == "#visible" => {
                            own.insert(target, Scalar::Bool(false));
                        }
                        None => {}
                    }
                }
                // Establishes the subtree cursor, which the factory already set;
                // custom renderers read the index it names.
                // Outside any grid of that collection the control stands for its first item.
                Some("collection_details") => {
                    if let Some(collection) = binding
                        .get("binding_collection_name")
                        .and_then(Value::as_str)
                    {
                        let index = scope.indices.get(collection).copied().unwrap_or(0);
                        own.insert("#collection_index".to_owned(), Scalar::Num(index as f64));
                        own.insert(
                            COLLECTION_NAME_KEY.to_owned(),
                            Scalar::Text(collection.to_owned()),
                        );
                    }
                }
                // `view` reads other controls' values; deferred to pass two.
                Some("view") => {}
                // `none` binds once at creation; stateless evaluation reads the global
                // when it exists and otherwise leaves the property unbound.
                Some("none") => {
                    let Some(source) = binding.get("binding_name").and_then(Value::as_str) else {
                        continue;
                    };
                    if let Some(value) = self.data.globals.get(source) {
                        own.insert(target_name(binding, source), value.clone());
                    }
                }
                Some("global") | None => {
                    let Some(source) = binding.get("binding_name").and_then(Value::as_str) else {
                        continue;
                    };
                    match lookup(
                        source,
                        &self.data.globals,
                        &own,
                        self.data.strict,
                        &self.env,
                    ) {
                        Some(value) => {
                            own.insert(target_name(binding, source), value);
                        }
                        // A controller answers a visibility flag it does not know
                        // with `false`; text and other values stay unbound.
                        None if self.data.strict && target_name(binding, source) == "#visible" => {
                            own.insert("#visible".to_owned(), Scalar::Bool(false));
                        }
                        None => {}
                    }
                }
                Some(_) => {}
            }
        }
        self.radio_state(control, &mut own);
        own
    }

    /// A radio-group toggle is checked when the screen selected its index; a
    /// `#name` forced index reads the toggle's own bound value (collection rows).
    fn radio_state(&self, control: &ResolvedControl, own: &mut BTreeMap<String, Scalar>) {
        if control.properties.get("radio_toggle_group") != Some(&Value::Bool(true)) {
            return;
        }
        let forced = control.properties.get("toggle_group_forced_index");
        let index = forced.and_then(Value::as_f64).or_else(|| {
            let name = forced.and_then(Value::as_str)?;
            match own.get(name)? {
                Scalar::Num(number) => Some(*number),
                Scalar::Text(text) => text.parse().ok(),
                Scalar::Bool(_) => None,
            }
        });
        let (Some(name), Some(index)) = (
            control
                .properties
                .get("toggle_name")
                .and_then(Value::as_str),
            index,
        ) else {
            return;
        };
        if let Some(Scalar::Num(selected)) = self.data.globals.get(&format!("#radio:{name}")) {
            own.insert("#toggle_state".to_owned(), Scalar::Bool(*selected == index));
        }
    }

    fn expand_factory(
        &mut self,
        control: &ResolvedControl,
        own: &BTreeMap<String, Scalar>,
        scope: &Scope,
    ) -> Vec<Node> {
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
            let Some(resolved) = self.resolve_scoped(&reference, control, &BTreeMap::new()) else {
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

    /// The data key for `name` in `scope`: a list registered for the innermost enclosing item wins
    /// over the shared plain-named list.
    fn collection_key(&self, name: &str, scope: &Scope) -> String {
        if let Some((parent, index)) = scope.path.last() {
            let scoped = scoped_key(parent, *index, name);
            if self.data.collections.contains_key(&scoped) {
                return scoped;
            }
        }
        name.to_owned()
    }

    /// Pass two: evaluate every `view` binding until the values settle, so a view
    /// may read another control's view-derived value (a title's stripped text).
    fn settle_views(&self, root: &mut Node) {
        for _ in 0..VIEW_PASSES {
            let mut updates = Vec::new();
            let names = first_by_name(root);
            self.collect_views(root, &names, &mut vec![root], &mut Vec::new(), &mut updates);
            let mut changed = false;
            for (path, name, value) in updates {
                let node = path
                    .iter()
                    .fold(&mut *root, |node, &index| &mut node.children[index]);
                if node.own.get(&name) != Some(&value) {
                    node.own.insert(name, value);
                    changed = true;
                }
            }
            if !changed {
                break;
            }
        }
    }

    /// `lineage` runs from the root down to the node being evaluated.
    fn collect_views<'n>(
        &self,
        node: &'n Node,
        names: &Names<'_>,
        lineage: &mut Vec<&'n Node>,
        path: &mut Vec<usize>,
        out: &mut Vec<(Vec<usize>, String, Scalar)>,
    ) {
        let has_views = bindings_of(node.src.get())
            .iter()
            .any(|binding| binding.get("binding_type").and_then(Value::as_str) == Some("view"));
        if has_views {
            let mut own = node.own.clone();
            self.resolve_views(lineage, names, &mut own);
            for (name, value) in own {
                if node.own.get(&name) != Some(&value) {
                    out.push((path.clone(), name, value));
                }
            }
        }
        for (index, child) in node.children.iter().enumerate() {
            path.push(index);
            lineage.push(child);
            self.collect_views(child, names, lineage, path, out);
            lineage.pop();
            path.pop();
        }
    }

    /// Bake binding-driven properties into literals.
    fn bake(&self, node: &Node) -> ResolvedControl {
        let control = node.src.get();
        let mut properties = bake_properties(&control.properties, &node.own);
        if let Some(patch) = &node.src.patch {
            properties.extend(
                patch
                    .properties
                    .iter()
                    .map(|(key, value)| (key.clone(), value.clone())),
            );
        }
        ResolvedControl {
            name: node.src.name().to_owned(),
            control_type: control.control_type.clone(),
            base: control.base.clone(),
            unresolved_base: control.unresolved_base.clone(),
            properties,
            children: node.children.iter().map(|child| self.bake(child)).collect(),
            factory: control.factory.clone(),
        }
    }

    /// Evaluate the last `lineage` node's `view` bindings into `own`. A named
    /// source is found breadth-first from the root, or from the parent with
    /// `resolve_sibling_scope`, or up the ancestors with `resolve_ancestor_scope`.
    fn resolve_views(
        &self,
        lineage: &[&Node],
        names: &Names<'_>,
        own: &mut BTreeMap<String, Scalar>,
    ) {
        let Some(node) = lineage.last() else {
            return;
        };
        for binding in bindings_of(node.src.get()) {
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
            let flag = |key: &str| binding.get(key).and_then(Value::as_bool) == Some(true);
            let snapshot;
            let mut layers: Vec<&BTreeMap<String, Scalar>> = Vec::new();
            match source {
                Some(name) => {
                    let found = if flag("resolve_ancestor_scope") {
                        lineage
                            .iter()
                            .rev()
                            .find(|ancestor| ancestor.src.name() == name)
                            .map(|ancestor| &ancestor.own)
                    } else if flag("resolve_sibling_scope") {
                        lineage
                            .len()
                            .checked_sub(2)
                            .and_then(|parent| breadth_first(lineage[parent], name))
                    } else {
                        names.get(name).copied()
                    };
                    layers.extend(found);
                }
                // The control's own values, then what its ancestors hold (a
                // screen factory's bag carries `#title_text` down to its form).
                None => {
                    snapshot = own.clone();
                    layers.push(&snapshot);
                    let ancestors = &lineage[..lineage.len() - 1];
                    layers.extend(ancestors.iter().rev().map(|ancestor| &ancestor.own));
                }
            }
            layers.push(&self.data.globals);
            let scope = ChainBindings(layers, self.data.strict);
            if let Some(value) = predicate::eval_scalar(expression, &self.env, &scope) {
                own.insert(target.to_owned(), value);
            }
        }
    }
}

/// Each name's first control in breadth-first order from the root, by values.
type Names<'a> = std::collections::HashMap<&'a str, &'a BTreeMap<String, Scalar>>;

fn first_by_name(root: &Node) -> Names<'_> {
    let mut names = Names::new();
    let mut queue = std::collections::VecDeque::from([root]);
    while let Some(node) = queue.pop_front() {
        names.entry(node.src.name()).or_insert(&node.own);
        queue.extend(node.children.iter());
    }
    names
}

/// The values of the first control named `name` in breadth-first order from `root`.
fn breadth_first<'a>(root: &'a Node, name: &str) -> Option<&'a BTreeMap<String, Scalar>> {
    let mut queue = std::collections::VecDeque::from([root]);
    while let Some(node) = queue.pop_front() {
        if node.src.name() == name {
            return Some(&node.own);
        }
        queue.extend(node.children.iter());
    }
    None
}

/// Rounds of `view` evaluation; each lets values flow one more control hop.
const VIEW_PASSES: usize = 4;
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

/// Replace `#`-referencing property values and binding-target properties with their
/// bound literals. Empty textures collapse to no property so no sprite is emitted;
/// an unbound `text` becomes empty rather than the literal `#name`.
fn bake_properties(
    properties: &BTreeMap<String, Value>,
    own: &BTreeMap<String, Scalar>,
) -> BTreeMap<String, Value> {
    let mut out = BTreeMap::new();
    for (key, value) in properties {
        // Only binding reads the created controls' scope.
        if key == crate::resolve::FACTORY_SCOPE {
            continue;
        }
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
    if let Some(alpha) = own.get("#alpha").and_then(scalar_number) {
        out.insert("alpha".to_owned(), scalar_to_value(&Scalar::Num(alpha)));
    }
    if let Some(propagate) = own.get("#propagateAlpha").and_then(Scalar::as_bool) {
        out.insert("propagate_alpha".to_owned(), Value::Bool(propagate));
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
    strict: bool,
    env: &Env,
) -> Option<Scalar> {
    if source.starts_with('(') {
        return predicate::eval_scalar(source, env, &LayeredBindings(values, own, strict));
    }
    values.get(source).cloned()
}

/// Values, then fallbacks; under strict semantics a name neither holds is `false`.
struct LayeredBindings<'a>(
    &'a BTreeMap<String, Scalar>,
    &'a BTreeMap<String, Scalar>,
    bool,
);

impl Bindings for LayeredBindings<'_> {
    fn get(&self, name: &str) -> Option<Scalar> {
        self.0
            .get(name)
            .or_else(|| self.1.get(name))
            .cloned()
            .or_else(|| self.2.then_some(Scalar::Bool(false)))
    }
}

/// Layers searched in order; under strict semantics a name none holds is `false`.
struct ChainBindings<'a>(Vec<&'a BTreeMap<String, Scalar>>, bool);

impl Bindings for ChainBindings<'_> {
    fn get(&self, name: &str) -> Option<Scalar> {
        self.0
            .iter()
            .find_map(|layer| layer.get(name))
            .cloned()
            .or_else(|| self.1.then_some(Scalar::Bool(false)))
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
fn with_index(src: Src, index: usize) -> Src {
    src.patched(|patch| {
        patch
            .properties
            .insert("collection_index".to_owned(), Value::from(index as u64));
    })
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
