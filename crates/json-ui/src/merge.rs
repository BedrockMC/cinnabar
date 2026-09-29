//! `@base` inheritance. A control is flattened by deep-merging its base chain,
//! child over base. Scalar and array properties are overridden by the child;
//! nested objects merge key by key; and `controls` merge by local instance name
//! (a same-named base control is overridden in place, new ones are appended). The
//! by-name rule matches vanilla templates such as `inactive_button@beacon.base_button`,
//! which overrides `default`/`hover` yet keeps the base's `pressed`.

use std::collections::HashSet;

use serde_json::{Map, Value};

use crate::catalog::{Catalog, RawControl};
use crate::tree::ControlRef;

/// Longest `@base` chain flattened; a longer server-supplied chain rejects the control.
const MAX_CHAIN: usize = 256;

/// Flatten `(ns, name)` along its literal `@base` chain into one control, with the
/// immediate base recorded for provenance. Returns `None` if the control is absent
/// or its chain exceeds [`MAX_CHAIN`]. A `$var` base is left for the caller.
pub fn flatten_def(
    catalog: &Catalog,
    namespace: &str,
    name: &str,
    diagnostics: &mut Vec<String>,
) -> Option<(RawControl, Option<ControlRef>)> {
    let top = catalog.lookup(namespace, name)?;
    let provenance = literal_base(top);
    let mut seen = HashSet::from([(namespace.to_owned(), name.to_owned())]);
    let mut chain = vec![top];
    while let Some(current) = chain.last().copied()
        && let Some(base_ref) = literal_base(current)
    {
        let label = format!("{}.{}", current.owner_ns, current.name);
        if !seen.insert((base_ref.namespace.clone(), base_ref.name.clone())) {
            diagnostics.push(format!("{label}: inheritance cycle through {base_ref}"));
            break;
        }
        let Some(base) = catalog.lookup(&base_ref.namespace, &base_ref.name) else {
            diagnostics.push(format!("{label}: base {base_ref} not found"));
            break;
        };
        if chain.len() >= MAX_CHAIN {
            diagnostics.push(format!(
                "{namespace}.{name}: inheritance chain longer than {MAX_CHAIN}; dropped"
            ));
            return None;
        }
        chain.push(base);
    }
    let mut chain = chain.into_iter().rev();
    let mut flattened = clear_base(chain.next()?.clone());
    for child in chain {
        flattened = clear_base(deep_merge_control(&flattened, child));
    }
    Some((flattened, provenance))
}

fn literal_base(control: &RawControl) -> Option<ControlRef> {
    let base = control
        .base
        .as_deref()
        .filter(|base| !base.starts_with('$'))?;
    Some(ControlRef::parse(base, &control.owner_ns))
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
    use super::{deep_merge_control, flatten_def};
    use crate::catalog::{Catalog, RawControl};
    use serde_json::{Value, json};

    /// `c0@ns.c1`, `c1@ns.c2`, ...; the last one points back at `c0` when `cyclic`.
    fn chain(len: usize, cyclic: bool) -> Catalog {
        let mut text = String::from(r#"{"namespace":"ns""#);
        for index in 0..len {
            let base = match (index + 1 < len, cyclic) {
                (true, _) => format!("@ns.c{}", index + 1),
                (false, true) => "@ns.c0".to_owned(),
                (false, false) => String::new(),
            };
            text.push_str(&format!(r#","c{index}{base}":{{"p{index}":{index}}}"#));
        }
        text.push('}');
        let mut catalog = Catalog::default();
        catalog.load_text("chain.json", &text);
        catalog
    }

    // A long server-supplied chain must be rejected, not overflow the stack.
    #[test]
    fn long_acyclic_inheritance_chain_is_rejected_with_a_diagnostic() {
        let mut diagnostics = Vec::new();
        let flattened = flatten_def(&chain(10_000, false), "ns", "c0", &mut diagnostics);
        assert!(flattened.is_none());
        assert!(
            diagnostics
                .iter()
                .any(|message| message.contains("inheritance chain"))
        );
    }

    #[test]
    fn bounded_chains_flatten_and_cycles_stop_at_the_repeat() {
        let mut diagnostics = Vec::new();
        let (control, base) =
            flatten_def(&chain(100, false), "ns", "c0", &mut diagnostics).expect("bounded chain");
        assert_eq!(control.props.len(), 100);
        assert_eq!(control.props.get("p99"), Some(&json!(99)));
        assert_eq!(base.map(|base| base.name), Some("c1".to_owned()));
        assert!(control.base.is_none() && diagnostics.is_empty());

        let (control, _) = flatten_def(&chain(3, true), "ns", "c0", &mut diagnostics)
            .expect("cyclic chain keeps the control");
        assert_eq!(control.props.len(), 3);
        assert!(
            diagnostics
                .iter()
                .any(|message| message.contains("inheritance cycle"))
        );
    }

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
