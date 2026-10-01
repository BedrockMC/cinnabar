//! Data binding as the client's `DataBindingComponent` runs it: each control's
//! property bag starts from its `property_bag` literals, its bindings write
//! controller values into it on the schedule their `binding_condition` sets,
//! `view` bindings observe other controls' bags, and every written target
//! reaches the component it drives. Factories and grids expand collections
//! into per-index instances. The result is a [`ResolvedControl`] tree with
//! bag values under their `#names` and bound component state as literals,
//! ready for [`crate::layout`]/[`crate::emit`].
//!
//! A [`DataSource`] is the screen controller: `global` values, per-index
//! `collection` items and named-factory feeds. A [`BindState`] is the live
//! controls between binds; binding without one is a fresh screen.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Arc;

use serde_json::Value;

use crate::env::Env;
use crate::predicate::Scalar;
use crate::tree::{ControlRef, Factory, ResolvedControl};

mod apply;
mod bag;
mod data;
mod feed;
mod native;
mod source;
mod spec;
mod state;
mod view;

pub use data::{CollectionItem, DataSource, scoped_key};
pub use feed::FactoryItem;
pub use state::BindState;

use bag::Bag;
use native::Native;
use source::Src;
use spec::Binding;
use state::Retained;

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

/// Bind `root` against `data` as a freshly created screen.
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

/// [`bind_shared`] plus what the binder skipped or corrected: rejected binding
/// declarations, factory roles with no control and unresolved templates.
pub fn bind_reporting(
    root: &Arc<ResolvedControl>,
    data: &DataSource,
    lib: &dyn ControlLibrary,
) -> (ResolvedControl, Vec<String>) {
    bind_with(root, data, lib, &mut BindState::new(), false)
}

/// One data refresh of a live screen: bindings run against `state`'s bags on
/// their schedules, and `state` keeps the result for the next refresh.
pub fn bind_stateful(
    root: &Arc<ResolvedControl>,
    data: &DataSource,
    lib: &dyn ControlLibrary,
    state: &mut BindState,
) -> (ResolvedControl, Vec<String>) {
    bind_with(root, data, lib, state, true)
}

fn bind_with(
    root: &Arc<ResolvedControl>,
    data: &DataSource,
    lib: &dyn ControlLibrary,
    state: &mut BindState,
    retain: bool,
) -> (ResolvedControl, Vec<String>) {
    let mut binder = Binder {
        data,
        lib,
        env: Env::new(),
        state,
        diagnostics: Vec::new(),
        reported: HashSet::new(),
        resolved: BTreeMap::new(),
        resolved_with: BTreeMap::new(),
        parsed: HashMap::new(),
        keys: state::KeyMap::default(),
        retain,
    };
    let mut node = binder.build(Src::root(Arc::clone(root)), &Scope::default());
    binder.settle_views(&mut node);
    let baked = binder.bake(&node);
    binder.retain(node);
    (baked, binder.diagnostics)
}

/// The collection a `collection_details` binding names, for hit regions.
pub(crate) const COLLECTION_NAME_KEY: &str = "#collection_name";

/// The collection cursors and inherited bags a control is created under.
#[derive(Clone, Debug)]
struct Scope {
    /// Shared until a control enters or attaches to a collection.
    cursor: Arc<Cursor>,
    /// The direct parent's `collection_name`, whose items its children are,
    /// and whether the parent is a grid.
    parent_collection: Option<(String, bool)>,
    /// A factory item's values, the base every control under it binds on.
    values: Arc<Bag>,
    /// The parent's `property_bag_for_children`.
    for_children: Arc<Bag>,
    /// The parent's layout-key hash.
    parent_key: u64,
    /// The nearest retained ancestor's key hash.
    retained_parent: u64,
}

impl Default for Scope {
    fn default() -> Self {
        Self {
            cursor: Arc::default(),
            parent_collection: None,
            values: Arc::default(),
            for_children: Arc::default(),
            parent_key: state::KEY_ROOT,
            retained_parent: state::KEY_ROOT,
        }
    }
}

#[derive(Clone, Debug, Default)]
struct Cursor {
    /// `collection_name` → the item index its `collection` bindings read;
    /// `-1` is an item without an index.
    indices: BTreeMap<String, i64>,
    /// The data key each active cursor's collection resolved to.
    keys: BTreeMap<String, String>,
    /// Entered data lists, outermost first: `(data key, index)`.
    path: Vec<(String, usize)>,
    /// Enclosing collection items, outermost first: `(collection, index)`.
    items: Vec<(String, i64)>,
}

impl Scope {
    /// The scope inside item `index` of `name`, whose list lives at `key`.
    fn enter(&self, name: &str, key: String, index: usize) -> Scope {
        let mut inner = self.clone();
        let cursor = Arc::make_mut(&mut inner.cursor);
        cursor.indices.insert(name.to_owned(), index as i64);
        cursor.keys.insert(name.to_owned(), key.clone());
        cursor.path.push((key, index));
        inner
    }
}

/// A control with its bag and component state for this refresh.
struct Node {
    src: Src,
    /// The layout-key hash, made unique among same-named siblings.
    key: u64,
    own: Bag,
    native: Native,
    memory: Retained,
    bindings: Arc<Vec<Binding>>,
    children: Vec<Node>,
    /// A hidden control's scope, kept to build its subtree once shown.
    deferred: Option<Scope>,
    /// Whether its state outlives the refresh.
    retained: bool,
}

struct Binder<'a> {
    data: &'a DataSource,
    lib: &'a dyn ControlLibrary,
    env: Env,
    state: &'a mut BindState,
    diagnostics: Vec<String>,
    reported: HashSet<String>,
    /// Library resolutions memoized per reference, shared by their instances.
    resolved: BTreeMap<ControlRef, Option<Arc<ResolvedControl>>>,
    /// Factory-item resolutions, keyed by reference and serialized `$vars`.
    resolved_with: BTreeMap<(ControlRef, String), Option<Arc<ResolvedControl>>>,
    /// Parsed binding declarations per template control.
    parsed: HashMap<usize, Arc<Vec<Binding>>>,
    /// Keys handed out this refresh, with how often.
    keys: state::KeyMap<usize>,
    /// Whether the refresh's state outlives it.
    retain: bool,
}

impl<'a> Binder<'a> {
    fn note(&mut self, message: String) {
        if self.reported.insert(message.clone()) {
            self.diagnostics.push(message);
        }
    }

    fn bindings_of(&mut self, control: &ResolvedControl) -> Arc<Vec<Binding>> {
        let id = control as *const ResolvedControl as usize;
        if let Some(parsed) = self.parsed.get(&id) {
            return Arc::clone(parsed);
        }
        let mut notes = Vec::new();
        let parsed = Arc::new(spec::parse(control, &mut notes));
        for note in notes {
            self.note(note);
        }
        self.parsed.insert(id, Arc::clone(&parsed));
        parsed
    }

    /// Create `src` under `scope`: its bag, its bindings for this refresh, and
    /// its subtree unless it is hidden.
    fn build(&mut self, src: Src, scope: &Scope) -> Node {
        let mut key = state::key_hash(scope.parent_key, b"/");
        key = state::key_hash(key, src.name().as_bytes());
        if let Some(index) = src.prop("collection_index").and_then(Value::as_u64) {
            key = state::key_hash(key, format!("[{index}]").as_bytes());
        }
        // Same-named siblings share a layout key; their binding state must not.
        let repeats = self.keys.entry(key).or_insert(0);
        *repeats += 1;
        if *repeats > 1 {
            key = state::key_hash(key, format!("~{repeats}").as_bytes());
        }
        let mut scope = scope.clone();
        self.attach_item(&src, &mut scope);
        let control = src.get();
        let (fresh, for_children) = bag::bags(control, &scope.for_children);
        let retained = self.state.controls.remove(&key);
        let parent = scope.retained_parent;
        let created = retained.is_none();
        let mut memory = retained.unwrap_or_default();
        memory.parent = parent;
        let mut own = if created {
            let mut own = (*scope.values).clone();
            own.extend(fresh);
            apply::widget_defaults(control, &mut own);
            own
        } else {
            let mut own = std::mem::take(&mut memory.bag);
            own.extend(
                scope
                    .values
                    .iter()
                    .map(|(name, value)| (name.clone(), value.clone())),
            );
            own
        };
        if let Some(values) = self.data.controls.get(src.name()) {
            own.extend(
                values
                    .iter()
                    .map(|(name, value)| (name.clone(), value.clone())),
            );
        }
        let published = self.state.published.remove(&key);
        let had_published = published.is_some();
        if let Some(published) = published {
            own.extend(published);
        }
        let mut native = Native {
            props: std::mem::take(&mut memory.native),
            collection_length: None,
        };
        let bindings = self.bindings_of(control);
        self.run_bindings(
            control,
            &bindings,
            &scope,
            &mut own,
            &mut native,
            &mut memory,
        );
        self.radio_state(control, &mut own);
        let src = native_grid(src, &native);
        let control = src.get();
        if let Some(cells) = grid_cells(&src) {
            own.insert("#grid_number_size".to_owned(), Scalar::Int(cells as i64));
        }
        let mut child_scope = scope.clone();
        child_scope.parent_key = key;
        // Only state a refresh cannot rebuild from literals is retained.
        let retained = !bindings.is_empty()
            || !native.props.is_empty()
            || !native.visible(control)
            || had_published;
        if retained {
            child_scope.retained_parent = key;
        }
        child_scope.for_children = for_children;
        child_scope.parent_collection = control
            .properties
            .get("collection_name")
            .and_then(Value::as_str)
            .filter(|name| !name.is_empty())
            .map(|name| {
                (
                    name.to_owned(),
                    control.control_type.as_deref() == Some("grid"),
                )
            });
        let mut node = Node {
            src,
            key,
            own,
            native,
            memory,
            bindings,
            children: Vec::new(),
            deferred: None,
            retained,
        };
        // A hidden control's subtree builds only once shown or named, so a
        // pack's many title-selected layouts cost only the one on screen.
        if !node.native.visible(node.src.get()) {
            node.deferred = Some(child_scope);
        } else {
            node.children = self.children_of(&node, &child_scope);
            if node.native.collection_length.is_some() {
                let created = node.children.len() as i64;
                node.own
                    .insert("#collection_number_size".to_owned(), Scalar::Int(created));
            }
        }
        node
    }

    /// `UIControlFactory`'s collection item: a child of a collection panel is
    /// an item at its `collection_index`, `-1` without one. A grid's cell is
    /// its grid item and keeps the enclosing index without one.
    fn attach_item(&self, src: &Src, scope: &mut Scope) {
        let Some((collection, grid)) = scope.parent_collection.take() else {
            return;
        };
        if src.prop("ignoreCollectionItem").and_then(Value::as_bool) == Some(true) {
            return;
        }
        let index = src
            .prop("collection_index")
            .and_then(Value::as_i64)
            .map(|index| index as i32 as i64)
            .filter(|index| *index != -1);
        let index = match (index, grid) {
            (Some(index), _) => index,
            (None, true) => return,
            (None, false) => -1,
        };
        let cursor = Arc::make_mut(&mut scope.cursor);
        cursor.indices.insert(collection.clone(), index);
        cursor.items.push((collection, index));
    }

    fn children_of(&mut self, node: &Node, scope: &Scope) -> Vec<Node> {
        let src = &node.src;
        let control = src.get();
        if is_collection_factory(control) {
            self.expand_factory(control, node, scope)
        } else if let Some(reference) = self.screen_factory(control) {
            self.resolve(&reference)
                .map(|resolved| vec![self.build(Src::root(resolved), scope)])
                .unwrap_or_default()
        } else if let Some(items) = self.feed(control) {
            self.expand_feed(control, items, scope)
        } else if let Some(template) = grid_template(control) {
            self.expand_grid(src, grid_cells(src), &template, scope)
        } else {
            let columns = static_grid_columns(src);
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

    /// Build deferred subtrees that are now shown under shown ancestors;
    /// `true` when any was built.
    fn expand_deferred(&mut self, node: &mut Node, parent_visible: bool) -> bool {
        if !parent_visible || !node.native.visible(node.src.get()) {
            return false;
        }
        let mut expanded = false;
        if let Some(scope) = node.deferred.take() {
            node.children = self.children_of(node, &scope);
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

    /// A radio-group toggle is checked when the screen selected its index; a
    /// `#name` forced index reads the toggle's own bound value (collection rows).
    fn radio_state(&self, control: &ResolvedControl, own: &mut Bag) {
        if control.properties.get("radio_toggle_group") != Some(&Value::Bool(true)) {
            return;
        }
        let forced = control.properties.get("toggle_group_forced_index");
        let index = forced.and_then(Value::as_f64).or_else(|| {
            let name = forced.and_then(Value::as_str)?;
            match own.get(name)? {
                Scalar::Text(text) => text.parse().ok(),
                other => other.as_number(),
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
        if let Some(selected) = self
            .data
            .globals
            .get(&format!("#radio:{name}"))
            .and_then(Scalar::as_number)
        {
            own.insert("#toggle_state".to_owned(), Scalar::Bool(selected == index));
        }
    }

    fn expand_factory(
        &mut self,
        control: &ResolvedControl,
        node: &Node,
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
        let roles: Vec<Option<String>> = match node.native.collection_length.as_ref() {
            Some(length) => bound_roles(length, factory),
            None => match self.data.collections.get(&key) {
                Some(items) => items.iter().map(|item| item.role.clone()).collect(),
                None => unsupplied_roles(factory, &node.own),
            },
        };
        let mut nodes = Vec::with_capacity(roles.len());
        for (index, role) in roles.iter().enumerate() {
            let role = role.as_deref();
            let Some(reference) = select_control(factory, role) else {
                self.note(format!(
                    "{}: factory has no control for role {role:?}",
                    control.name
                ));
                continue;
            };
            let reference = reference.clone();
            let Some(resolved) = self.resolve_scoped(&reference, control, &BTreeMap::new()) else {
                self.note(format!(
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
        if let Some((parent, index)) = scope.cursor.path.last() {
            let scoped = scoped_key(parent, *index, name);
            if self.data.collections.contains_key(&scoped) {
                return scoped;
            }
        }
        name.to_owned()
    }

    /// One `grid_item_template` instance per collection item, capped by
    /// `maximum_grid_items` when set.
    fn expand_grid(
        &mut self,
        src: &Src,
        cells: Option<u64>,
        template: &ControlRef,
        scope: &Scope,
    ) -> Vec<Node> {
        let control = src.get();
        let Some(collection) = control
            .properties
            .get("collection_name")
            .and_then(Value::as_str)
        else {
            return Vec::new();
        };
        let cap = src
            .prop("maximum_grid_items")
            .and_then(Value::as_u64)
            .filter(|cap| *cap > 0)
            .map_or(usize::MAX, |cap| cap as usize);
        // A fixed grid always has `columns * rows` cells; a rescaling one follows
        // its collection.
        let key = self.collection_key(collection, scope);
        let count = cells
            .map_or_else(|| self.data.collection_len(&key), |cells| cells as usize)
            .min(cap);
        let Some(resolved) = self.resolve_scoped(template, control, &BTreeMap::new()) else {
            self.note(format!(
                "{}: grid template {template} unresolved",
                control.name
            ));
            return Vec::new();
        };
        (0..count)
            .map(|index| {
                let child_scope = scope.enter(collection, key.clone(), index);
                let cell = with_index(Src::root(Arc::clone(&resolved)), index).patched(|patch| {
                    patch.properties.insert(
                        "collection_scope".to_owned(),
                        Value::String(collection.to_owned()),
                    );
                });
                self.build(cell, &child_scope)
            })
            .collect()
    }

    /// Bake bag values and bound component state into literals.
    fn bake(&self, node: &Node) -> ResolvedControl {
        let control = node.src.get();
        let mut properties = bake_properties(&control.properties, &node.own);
        properties.extend(
            node.native
                .props
                .iter()
                .map(|(key, value)| (key.clone(), value.clone())),
        );
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

    /// Keep every built control's bag and binding memory for the next refresh,
    /// and the memory of controls inside subtrees still hidden.
    fn retain(&mut self, node: Node) {
        if !self.retain {
            return;
        }
        self.state.generation += 1;
        let generation = self.state.generation;
        let leftover = std::mem::take(&mut self.state.controls);
        self.keep(node, generation);
        // A control not built this refresh stays only under a still-hidden one.
        let built = &self.state.controls;
        let kept: Vec<u64> = leftover
            .iter()
            .filter(|(_, memory)| {
                let mut parent = memory.parent;
                for _ in 0..MAX_KEEP_DEPTH {
                    if let Some(control) = built.get(&parent) {
                        return control.deferred;
                    }
                    match leftover.get(&parent) {
                        Some(above) => parent = above.parent,
                        None => return false,
                    }
                }
                false
            })
            .map(|(key, _)| *key)
            .collect();
        let mut leftover = leftover;
        for key in kept {
            if let Some(memory) = leftover.remove(&key) {
                self.state.controls.insert(key, memory);
            }
        }
    }

    fn keep(&mut self, node: Node, generation: u64) {
        let Node {
            key,
            own,
            native,
            mut memory,
            children,
            deferred,
            retained,
            ..
        } = node;
        for child in children {
            self.keep(child, generation);
        }
        if !retained {
            return;
        }
        memory.bag = own;
        memory.native = native.props;
        memory.generation = generation;
        memory.deferred = deferred.is_some();
        self.state.controls.insert(key, memory);
    }
}

/// Ancestors a hidden control's memory is traced through.
const MAX_KEEP_DEPTH: usize = 256;

/// A grid's bound dimensions and item cap as the literals its expansion reads.
fn native_grid(src: Src, native: &Native) -> Src {
    let bound: Vec<(&String, &Value)> = native
        .props
        .iter()
        .filter(|(key, _)| matches!(key.as_str(), "grid_dimensions" | "maximum_grid_items"))
        .collect();
    if bound.is_empty() {
        return src;
    }
    let bound: Vec<(String, Value)> = bound
        .into_iter()
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect();
    src.patched(|patch| patch.properties.extend(bound))
}

/// A fixed grid's `columns * rows`, which `GridComponent` publishes.
fn grid_cells(src: &Src) -> Option<u64> {
    if src.get().control_type.as_deref() != Some("grid") {
        return None;
    }
    let dims = src.prop("grid_dimensions")?.as_array()?;
    Some(dims.first()?.as_u64()? * dims.get(1)?.as_u64()?)
}

/// Most instances a factory makes from a bound or literal count.
const MAX_FACTORY_ITEMS: usize = 4096;

/// Roles for a bound `#collection_length`: one per control id, or that many
/// of a `control_name` template.
fn bound_roles(length: &Value, factory: &Factory) -> Vec<Option<String>> {
    let cap = factory.max_children_size.unwrap_or(MAX_FACTORY_ITEMS);
    match length {
        Value::Array(ids) => ids
            .iter()
            .take(cap)
            .map(|id| id.as_str().map(str::to_owned))
            .collect(),
        other => {
            let count = other.as_i64().unwrap_or(0).max(0) as usize;
            vec![None; count.min(cap)]
        }
    }
}

/// Roles for a collection the screen does not supply, from a literal
/// `#collection_length`: an array of control ids makes one instance per id; a
/// number makes that many only for a `control_name` template.
fn unsupplied_roles(factory: &Factory, own: &Bag) -> Vec<Option<String>> {
    match own.get("#collection_length") {
        Some(Scalar::Json(Value::Array(ids))) => ids
            .iter()
            .take(MAX_FACTORY_ITEMS)
            .map(|id| id.as_str().map(str::to_owned))
            .collect(),
        Some(length) if factory.control_name.is_some() => {
            let count = length.as_number().unwrap_or(0.0).max(0.0) as usize;
            vec![None; count.min(MAX_FACTORY_ITEMS)]
        }
        _ => Vec::new(),
    }
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

/// Replace `#`-referencing property values with their bag values and carry the
/// bag's `#names`. An unbound `text` becomes empty rather than the literal
/// `#name`; a `##` text is literal.
fn bake_properties(properties: &BTreeMap<String, Value>, own: &Bag) -> BTreeMap<String, Value> {
    let mut out = BTreeMap::new();
    for (key, value) in properties {
        // Only binding reads the created controls' scope.
        if key == crate::resolve::FACTORY_SCOPE {
            continue;
        }
        match value {
            Value::String(reference)
                if reference.starts_with('#')
                    && !(key == "text" && reference.starts_with("##")) =>
            {
                match own.get(reference) {
                    Some(scalar) => {
                        out.insert(key.clone(), scalar.to_json());
                    }
                    None if key == "text" => {
                        out.insert(key.clone(), Value::String(String::new()));
                    }
                    None => {}
                }
            }
            _ => {
                out.insert(key.clone(), value.clone());
            }
        }
    }
    for (name, value) in own {
        if name.starts_with('#') {
            out.insert(name.clone(), value.to_json());
        }
    }
    out
}

/// The column count of a collection grid that lists its cells as children.
fn static_grid_columns(src: &Src) -> Option<u64> {
    let control = src.get();
    if control.control_type.as_deref() != Some("grid")
        || !control.properties.contains_key("collection_name")
    {
        return None;
    }
    src.prop("grid_dimensions")?
        .as_array()?
        .first()?
        .as_u64()
        .filter(|columns| *columns > 0)
}

/// A static grid cell's collection index, row-major from its `grid_position`.
fn grid_cell_index(control: &ResolvedControl, columns: u64) -> Option<usize> {
    if control.properties.contains_key("collection_index") {
        return None;
    }
    let position = control.properties.get("grid_position")?.as_array()?;
    let (column, row) = (position.first()?.as_u64()?, position.get(1)?.as_u64()?);
    usize::try_from(row * columns + column).ok()
}

/// A factory/grid instance records its collection index for keys and events.
fn with_index(src: Src, index: usize) -> Src {
    src.patched(|patch| {
        patch
            .properties
            .insert("collection_index".to_owned(), Value::from(index as u64));
    })
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
