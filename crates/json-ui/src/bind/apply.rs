//! `DataBindingComponent::_bind` for one control: each binding runs when its
//! condition allows, the controller writes its properties into the bag, and
//! the written target reaches its component.

use serde_json::{Map, Value};

use super::bag::Bag;
use super::native::{self, Native};
use super::spec::{Binding, Condition, Kind, Source};
use super::state::Retained;
use super::{Binder, Scope};
use crate::predicate::{self, Scalar};
use crate::tree::ResolvedControl;

impl Binder<'_> {
    /// Run `control`'s non-view bindings on their schedules.
    pub(super) fn run_bindings(
        &self,
        control: &ResolvedControl,
        bindings: &[Binding],
        scope: &Scope,
        own: &mut Bag,
        native: &mut Native,
        memory: &mut Retained,
    ) {
        // The control's own visible flag as it stood before this refresh.
        let visible = native.visible(control);
        for (index, binding) in bindings.iter().enumerate() {
            if matches!(binding.kind, Kind::View { .. }) {
                continue;
            }
            let due = match binding.condition {
                Condition::None | Condition::Always => true,
                Condition::Visible | Condition::AlwaysWhenVisible => visible,
                Condition::VisibilityChanged => {
                    let changed = memory.seen.get(&index).copied().unwrap_or(false) != visible;
                    memory.seen.insert(index, visible);
                    changed
                }
                Condition::Once => {
                    let waiting = binding
                        .collection()
                        .is_some_and(|name| scope.cursor.indices.get(name) == Some(&-1));
                    let due = !memory.once.contains(&index) && !waiting;
                    if due {
                        memory.once.insert(index);
                    }
                    due
                }
            };
            if due {
                self.run(control, binding, scope, own, native);
            }
        }
    }

    fn run(
        &self,
        control: &ResolvedControl,
        binding: &Binding,
        scope: &Scope,
        own: &mut Bag,
        native: &mut Native,
    ) {
        match &binding.kind {
            Kind::Global { source, rename } => {
                let data = self.data;
                self.query(control, source, rename, own, native, |name| {
                    data.globals.get(name).cloned()
                });
            }
            Kind::Collection {
                source,
                rename,
                collection,
            } => {
                if collection.is_empty() {
                    return;
                }
                // Outside any item of that collection a control reads item 0.
                let index = scope.cursor.indices.get(collection).copied().unwrap_or(0);
                if index < 0 {
                    return;
                }
                let key = scope
                    .cursor
                    .keys
                    .get(collection)
                    .map_or(collection.as_str(), String::as_str);
                let item = self
                    .data
                    .collections
                    .get(key)
                    .and_then(|items| items.get(index as usize));
                self.query(control, source, rename, own, native, |name| {
                    item?.values.get(name).cloned()
                });
            }
            Kind::Details { collection, prefix } => details(collection, prefix, scope, own),
            Kind::View { .. } => {}
        }
    }

    /// Ask the controller for each property `source` reads, writing answers
    /// under the target, then apply the target through the binding's
    /// expression rewritten to read it.
    fn query(
        &self,
        control: &ResolvedControl,
        source: &Source,
        rename: &str,
        own: &mut Bag,
        native: &mut Native,
        answer: impl Fn(&str) -> Option<Scalar>,
    ) {
        let strict = self.data.strict;
        for (rank, property) in source.properties().into_iter().enumerate() {
            let target = if rename.is_empty() { property } else { rename };
            match answer(property) {
                Some(value) => {
                    own.insert(target.to_owned(), value);
                }
                // A controller answers a visibility or checked flag, or an
                // expression operand, it does not know with `false`.
                None if strict
                    && (matches!(target, "#visible" | "#toggle_state")
                        || matches!(source, Source::Expression { .. })) =>
                {
                    own.insert(target.to_owned(), Scalar::Bool(false));
                }
                None => {}
            }
            let value = match source {
                Source::Simple(_) => own.get(target).map_or(Value::Null, Scalar::to_json),
                Source::Expression { text, .. } => {
                    predicate::eval_rewritten(text, rank + 1, rename, &self.env, &BagScope(own))
                        .map_or(Value::Null, |value| value.to_json())
                }
            };
            native::apply(target, &value, control, own, native);
        }
    }
}

/// A bag read as an expression's property scope.
pub(super) struct BagScope<'a>(pub(super) &'a Bag);

impl predicate::Bindings for BagScope<'_> {
    fn get(&self, name: &str) -> Option<Scalar> {
        self.0.get(name).cloned()
    }
}

/// `collection_details`: the named collection's item index and name, or every
/// enclosing collection's index as one object, under `#<prefix>_`.
fn details(collection: &str, prefix: &str, scope: &Scope, own: &mut Bag) {
    let prefix = if prefix.is_empty() {
        String::new()
    } else {
        format!("{prefix}_")
    };
    if collection.is_empty() {
        let mut map = Map::new();
        for (name, index) in &scope.cursor.items {
            map.entry(format!("{prefix}{name}"))
                .or_insert_with(|| Value::from(*index));
        }
        own.insert(
            format!("#{prefix}collections"),
            Scalar::Json(Value::Object(map)),
        );
        return;
    }
    let index = scope.cursor.indices.get(collection).copied().unwrap_or(0);
    own.insert(
        format!("#{prefix}collection_name"),
        Scalar::Text(collection.to_owned()),
    );
    own.insert(format!("#{prefix}collection_index"), Scalar::Int(index));
}

/// Bag values a widget component publishes when created, before any binding.
pub(super) fn widget_defaults(control: &ResolvedControl, own: &mut Bag) {
    if control.control_type.as_deref() == Some("scrollbar_box") {
        own.insert("#is_scroll_bar_box".to_owned(), Scalar::Bool(true));
    }
    if control.control_type.as_deref() == Some("toggle") {
        let checked = control
            .properties
            .get("toggle_default_state")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        own.entry("#toggle_state".to_owned())
            .or_insert(Scalar::Bool(checked));
    }
}
