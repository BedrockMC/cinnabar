//! A control's property bags at creation, as `UIControl::processPropertyBags`
//! builds them: its own `property_bag` and `property_bag_for_children`, each
//! member evaluated, and the parent's children bag merged into both without
//! overwriting.

use std::collections::BTreeMap;
use std::sync::Arc;

use serde_json::Value;

use crate::env::Env;
use crate::predicate::{self, NoBindings, Scalar};
use crate::tree::ResolvedControl;

pub(super) type Bag = BTreeMap<String, Scalar>;

/// `(own bag, children bag)` for `control` under a parent children bag.
pub(super) fn bags(control: &ResolvedControl, inherited: &Arc<Bag>) -> (Bag, Arc<Bag>) {
    let mut own = members(control.properties.get("property_bag"));
    let mut children = members(control.properties.get("property_bag_for_children"));
    for (name, value) in inherited.iter() {
        own.entry(name.clone()).or_insert_with(|| value.clone());
    }
    let children = if children.is_empty() {
        Arc::clone(inherited)
    } else {
        for (name, value) in inherited.iter() {
            children
                .entry(name.clone())
                .or_insert_with(|| value.clone());
        }
        Arc::new(children)
    };
    (own, children)
}

/// A bag literal's members, each through `UIResolvedDef::_evaluate`: a
/// parenthesised expression that reads no property becomes its value.
fn members(value: Option<&Value>) -> Bag {
    let Some(Value::Object(members)) = value else {
        return Bag::new();
    };
    members
        .iter()
        .map(|(name, value)| (name.clone(), member(value)))
        .collect()
}

fn member(value: &Value) -> Scalar {
    if let Value::String(text) = value
        && text.trim_start().starts_with('(')
        && predicate::property_tokens(text).is_empty()
        && let Some(result) = predicate::eval_scalar(text, &Env::new(), &NoBindings)
    {
        return result;
    }
    Scalar::from_json(value)
}
