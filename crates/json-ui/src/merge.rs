//! `@base` inheritance. A control is flattened by deep-merging its base chain,
//! child over base. Scalar and array properties are overridden by the child;
//! nested objects merge key by key; and `controls` merge by local instance name
//! (a same-named base control is overridden in place, new ones are appended). The
//! by-name rule matches vanilla templates such as `inactive_button@beacon.base_button`,
//! which overrides `default`/`hover` yet keeps the base's `pressed`.

use serde_json::{Map, Value};

use crate::catalog::{Catalog, RawControl};
use crate::tree::ControlRef;

/// Flatten `(ns, name)` along its literal `@base` chain into one control, with the
/// immediate base recorded for provenance. Returns `None` if the control is absent.
/// A base that is a `$var` is left for the caller to resolve against an env.
pub fn flatten_def(
    catalog: &Catalog,
    namespace: &str,
    name: &str,
    visited: &mut Vec<(String, String)>,
    diagnostics: &mut Vec<String>,
) -> Option<(RawControl, Option<ControlRef>)> {
    let control = catalog.lookup(namespace, name)?;
    let Some(base) = &control.base else {
        return Some((clear_base(control.clone()), None));
    };
    if base.starts_with('$') {
        return Some((clear_base(control.clone()), None));
    }
    let base_ref = ControlRef::parse(base, &control.owner_ns);
    let key = (base_ref.namespace.clone(), base_ref.name.clone());
    if visited.contains(&key) {
        diagnostics.push(format!(
            "{namespace}.{name}: inheritance cycle through {base_ref}"
        ));
        return Some((clear_base(control.clone()), Some(base_ref)));
    }
    visited.push(key);
    let flattened = match flatten_def(
        catalog,
        &base_ref.namespace,
        &base_ref.name,
        visited,
        diagnostics,
    ) {
        Some((base_control, _)) => deep_merge_control(&base_control, control),
        None => {
            diagnostics.push(format!("{namespace}.{name}: base {base_ref} not found"));
            control.clone()
        }
    };
    visited.pop();
    Some((clear_base(flattened), Some(base_ref)))
}

/// Merge `child` onto `base`, producing a control that keeps `child`'s identity.
pub fn deep_merge_control(base: &RawControl, child: &RawControl) -> RawControl {
    RawControl {
        owner_ns: child.owner_ns.clone(),
        name: child.name.clone(),
        base: child.base.clone().or_else(|| base.base.clone()),
        props: deep_merge_map(&base.props, &child.props),
        children: merge_children(&base.children, &child.children),
    }
}

fn clear_base(mut control: RawControl) -> RawControl {
    control.base = None;
    control
}

fn deep_merge_map(base: &Map<String, Value>, child: &Map<String, Value>) -> Map<String, Value> {
    let mut merged = base.clone();
    for (key, value) in child {
        match (merged.get(key), value) {
            (Some(Value::Object(base_object)), Value::Object(child_object)) => {
                merged.insert(
                    key.clone(),
                    Value::Object(deep_merge_map(base_object, child_object)),
                );
            }
            _ => {
                merged.insert(key.clone(), value.clone());
            }
        }
    }
    merged
}

fn merge_children(base: &[RawControl], child: &[RawControl]) -> Vec<RawControl> {
    let mut merged = base.to_vec();
    for incoming in child {
        match merged
            .iter()
            .position(|existing| existing.name == incoming.name)
        {
            Some(index) => merged[index] = deep_merge_control(&merged[index], incoming),
            None => merged.push(incoming.clone()),
        }
    }
    merged
}

#[cfg(test)]
mod tests {
    use super::deep_merge_control;
    use crate::catalog::RawControl;
    use serde_json::{Value, json};

    fn control(
        name: &str,
        base: Option<&str>,
        props: Value,
        children: Vec<RawControl>,
    ) -> RawControl {
        let Value::Object(props) = props else {
            unreachable!()
        };
        RawControl {
            owner_ns: "ns".to_owned(),
            name: name.to_owned(),
            base: base.map(str::to_owned),
            props,
            children,
        }
    }

    fn leaf(name: &str, base: Option<&str>) -> RawControl {
        control(name, base, json!({}), Vec::new())
    }

    #[test]
    fn child_overrides_scalars_and_keeps_base_only_properties() {
        let base = control(
            "btn",
            None,
            json!({ "size": [1, 1], "color": "base" }),
            Vec::new(),
        );
        let child = control(
            "btn",
            Some("ns.base"),
            json!({ "size": [2, 2] }),
            Vec::new(),
        );
        let merged = deep_merge_control(&base, &child);
        assert_eq!(merged.props.get("size"), Some(&json!([2, 2])));
        assert_eq!(merged.props.get("color"), Some(&json!("base")));
    }

    #[test]
    fn controls_merge_by_name_overriding_and_appending() {
        let base = control(
            "btn",
            None,
            json!({}),
            vec![
                leaf("default", None),
                leaf("hover", None),
                leaf("pressed", None),
            ],
        );
        let child = control(
            "btn",
            None,
            json!({}),
            vec![
                leaf("default", None),
                leaf("hover", Some("ns.hover_state")),
                leaf("extra", None),
            ],
        );
        let merged = deep_merge_control(&base, &child);
        let names: Vec<&str> = merged.children.iter().map(|c| c.name.as_str()).collect();
        // pressed survives (only in base), extra appends, base order is kept.
        assert_eq!(names, ["default", "hover", "pressed", "extra"]);
        // the overridden `hover` slot takes the child's new base.
        let hover = merged.children.iter().find(|c| c.name == "hover").unwrap();
        assert_eq!(hover.base.as_deref(), Some("ns.hover_state"));
    }

    #[test]
    fn nested_objects_merge_key_by_key() {
        let base = control("p", None, json!({ "bag": { "a": 1, "b": 2 } }), Vec::new());
        let child = control("p", None, json!({ "bag": { "b": 3, "c": 4 } }), Vec::new());
        let merged = deep_merge_control(&base, &child);
        assert_eq!(
            merged.props.get("bag"),
            Some(&json!({ "a": 1, "b": 3, "c": 4 }))
        );
    }
}
