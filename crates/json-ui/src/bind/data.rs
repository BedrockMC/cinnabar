//! The screen data source a form binds against: globals, collections and
//! named-factory feeds a screen controller supplies.

use std::collections::BTreeMap;

use super::FactoryItem;
use crate::predicate::Scalar;
use crate::tree::ResolvedControl;

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
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DataSource {
    pub(super) globals: BTreeMap<String, Scalar>,
    pub(super) collections: BTreeMap<String, Vec<CollectionItem>>,
    /// Controls created through named factories (`chat_item_factory`, …).
    pub(super) factories: BTreeMap<String, Vec<FactoryItem>>,
    /// `[columns, rows]` a `grid_dimension_binding` reads.
    pub(super) grid_dimensions: BTreeMap<String, [u32; 2]>,
    /// Screen-controller semantics: an unbound `#name` reads as `false` rather
    /// than leaving the template's literal in place.
    pub(super) strict: bool,
    /// The control id a screen's collection-less `factory` instantiates.
    pub(super) factory_id: Option<String>,
    /// Bag values components published on named controls last frame.
    pub(super) control_values: BTreeMap<String, BTreeMap<String, Scalar>>,
    /// What the screen's components wrote into their controls' bags.
    pub(super) components: crate::component::Components,
}

impl DataSource {
    pub fn new() -> Self {
        Self::default()
    }

    /// Bind over what the screen's components wrote into their bags.
    pub fn set_components(&mut self, components: crate::component::Components) {
        self.components = components;
    }

    /// Set a `global` binding value, keyed with its leading `#`.
    pub fn set_global(&mut self, name: impl Into<String>, value: Scalar) {
        self.globals.insert(name.into(), value);
    }

    /// Read unbound globals as `false`, as a screen controller answers bindings
    /// it does not provide. Menu screens bind this way; forms stay lenient.
    pub fn set_strict(&mut self, strict: bool) {
        self.strict = strict;
    }

    /// Select `index` in the radio toggle group named `toggle_name`: the toggle
    /// whose `toggle_group_forced_index` matches reads as checked.
    pub fn select_radio(&mut self, toggle_name: &str, index: usize) {
        self.globals
            .insert(format!("#radio:{toggle_name}"), Scalar::Num(index as f64));
    }

    /// Publish `values` in the bag of controls named `name`, as their components
    /// do (a scroll view's `#scrolled_to_end`), for `view` bindings to read.
    pub fn set_control_values(
        &mut self,
        name: impl Into<String>,
        values: BTreeMap<String, Scalar>,
    ) {
        self.control_values.insert(name.into(), values);
    }

    /// Select the `control_ids` entry a collection-less factory instantiates, as a
    /// screen controller picks its content (`long_form`, `custom_form`).
    pub fn set_factory_id(&mut self, id: impl Into<String>) {
        self.factory_id = Some(id.into());
    }

    /// Replace a named collection's per-index items.
    pub fn set_collection(&mut self, name: impl Into<String>, items: Vec<CollectionItem>) {
        self.collections.insert(name.into(), items);
    }

    /// The controls the factory named `name` holds, oldest first.
    pub fn set_factory(&mut self, name: impl Into<String>, items: Vec<FactoryItem>) {
        self.factories.insert(name.into(), items);
    }

    /// Answer a `grid_dimension_binding` named `name` (with its `#`).
    pub fn set_grid_dimensions(&mut self, name: impl Into<String>, dimensions: [u32; 2]) {
        self.grid_dimensions.insert(name.into(), dimensions);
    }

    /// Replace the list a collection named `name` reads while inside item `index` of the enclosing
    /// list stored at `parent_key` (a plain name, or another scoped key).
    pub fn set_scoped_collection(
        &mut self,
        parent_key: &str,
        index: usize,
        name: &str,
        items: Vec<CollectionItem>,
    ) {
        self.collections
            .insert(scoped_key(parent_key, index, name), items);
    }

    pub(super) fn collection_len(&self, name: &str) -> usize {
        self.collections.get(name).map_or(0, Vec::len)
    }
}

/// The data key of collection `name` inside item `index` of the list at `parent_key`.
pub fn scoped_key(parent_key: &str, index: usize, name: &str) -> String {
    format!("{parent_key}[{index}].{name}")
}

impl DataSource {
    /// What components published in the bag of controls named `name`.
    pub(super) fn control_values(&self, name: &str) -> impl Iterator<Item = (String, Scalar)> + '_ {
        self.control_values
            .get(name)
            .into_iter()
            .flatten()
            .map(|(key, value)| (key.clone(), value.clone()))
    }
}

/// What a control's components write into their bag on creation: a scroll
/// view's end flags and a scrollbar box's marker.
pub(super) fn component_bag(control: &ResolvedControl) -> Vec<(String, Scalar)> {
    match control.control_type.as_deref() {
        Some("scroll_view") => vec![
            ("#scrollbar_hit_bottom".to_owned(), Scalar::Bool(false)),
            ("#scrolled_to_end".to_owned(), Scalar::Bool(true)),
        ],
        Some("scrollbar_box") => vec![("#is_scroll_bar_box".to_owned(), Scalar::Bool(true))],
        _ => Vec::new(),
    }
}
