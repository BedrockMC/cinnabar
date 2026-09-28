//! Stage C: turn raw controls into the resolved tree. For each control we build a
//! variable scope (inherited scope, then `$decl`s, then matching `variables[]`
//! blocks), substitute `$vars` in properties, record any factory, drop `ignored`
//! children, and recurse. Base references on child keys are resolved here because
//! they may be `$var`s that only the scope knows.

use serde_json::{Map, Value};

use crate::catalog::{Catalog, RawControl};
use crate::env::{Env, apply_declarations, fold_expression, parse_var_key, substitute};
use crate::merge::{deep_merge_control, flatten_def};
use crate::predicate;
use crate::tree::{ControlRef, Factory, ResolvedControl};

const MAX_DEPTH: usize = 256;
const MAX_NODES: usize = 200_000;

/// Drives resolution over one [`Catalog`], accumulating diagnostics.
pub struct Resolver<'a> {
    catalog: &'a Catalog,
    diagnostics: Vec<String>,
    nodes: usize,
}

impl<'a> Resolver<'a> {
    pub fn new(catalog: &'a Catalog) -> Self {
        Self {
            catalog,
            diagnostics: Vec::new(),
            nodes: 0,
        }
    }

    pub fn diagnostics(&self) -> &[String] {
        &self.diagnostics
    }

    /// Record a caller-level diagnostic.
    pub fn note(&mut self, message: String) {
        self.diagnostics.push(message);
    }

    pub fn into_diagnostics(self) -> Vec<String> {
        self.diagnostics
    }

    /// Resolve `namespace.name` under `root_env` (globals plus context flags).
    pub fn resolve(
        &mut self,
        namespace: &str,
        name: &str,
        root_env: &Env,
    ) -> Option<ResolvedControl> {
        let (control, provenance) = flatten_def(
            self.catalog,
            namespace,
            name,
            &mut Vec::new(),
            &mut self.diagnostics,
        )?;
        let env = self.build_env(root_env, &control.props);
        Some(self.resolve_with_env(&control, provenance, None, &env, 0))
    }

    fn resolve_with_env(
        &mut self,
        control: &RawControl,
        provenance: Option<ControlRef>,
        unresolved_base: Option<String>,
        env: &Env,
        depth: usize,
    ) -> ResolvedControl {
        self.nodes += 1;
        let mut missing = Vec::new();
        let control_type = control
            .props
            .get("type")
            .map(|value| substitute(value, env, &mut missing))
            .and_then(value_string);
        let (factory, control_ids_consumed) =
            self.extract_factory(control, control_type.as_deref(), env);
        let properties = build_properties(control, env, control_ids_consumed, &mut missing);
        if !missing.is_empty() {
            missing.sort();
            missing.dedup();
            self.diagnostics.push(format!(
                "{}.{}: unresolved $vars: {}",
                control.owner_ns,
                control.name,
                missing.join(", ")
            ));
        }
        let children = self.resolve_children(control, env, depth);
        ResolvedControl {
            name: control.name.clone(),
            control_type,
            base: provenance,
            unresolved_base,
            properties,
            children,
            factory,
        }
    }

    fn resolve_children(
        &mut self,
        control: &RawControl,
        env: &Env,
        depth: usize,
    ) -> Vec<ResolvedControl> {
        if depth >= MAX_DEPTH || self.nodes >= MAX_NODES {
            if depth >= MAX_DEPTH {
                self.diagnostics.push(format!(
                    "{}.{}: max depth reached",
                    control.owner_ns, control.name
                ));
            }
            return Vec::new();
        }
        let mut resolved = Vec::new();
        for child in &control.children {
            let (working, provenance, unresolved) = self.resolve_child_base(child, env);
            let child_env = self.build_env(env, &working.props);
            if self.is_ignored(&working, &child_env) {
                continue;
            }
            resolved.push(self.resolve_with_env(
                &working,
                provenance,
                unresolved,
                &child_env,
                depth + 1,
            ));
        }
        resolved
    }

    fn is_ignored(&mut self, control: &RawControl, env: &Env) -> bool {
        match control.props.get("ignored") {
            None => false,
            Some(Value::Bool(flag)) => *flag,
            Some(Value::String(expression)) => match predicate::eval(expression, env) {
                Some(flag) => flag,
                None => {
                    self.diagnostics.push(format!(
                        "{}.{}: undecidable `ignored` `{expression}`; keeping",
                        control.owner_ns, control.name
                    ));
                    false
                }
            },
            Some(_) => false,
        }
    }

    /// Resolve a child's `@base` (literal or `$var`) and merge it under the child.
    fn resolve_child_base(
        &mut self,
        child: &RawControl,
        env: &Env,
    ) -> (RawControl, Option<ControlRef>, Option<String>) {
        let Some(base) = &child.base else {
            return (child.clone(), None, None);
        };
        let reference = match base.strip_prefix('$') {
            Some(variable) => match env.get(variable) {
                Some(Value::String(text)) => text.clone(),
                _ => {
                    let mut cleared = child.clone();
                    cleared.base = None;
                    return (cleared, None, Some(base.clone()));
                }
            },
            None => base.clone(),
        };
        let base_ref = ControlRef::parse(&reference, &child.owner_ns);
        let mut visited = Vec::new();
        match flatten_def(
            self.catalog,
            &base_ref.namespace,
            &base_ref.name,
            &mut visited,
            &mut self.diagnostics,
        ) {
            Some((base_control, _)) => {
                let mut working = deep_merge_control(&base_control, child);
                working.base = None;
                (working, Some(base_ref), None)
            }
            None => {
                self.diagnostics.push(format!(
                    "{}.{}: base {base_ref} not found",
                    child.owner_ns, child.name
                ));
                let mut cleared = child.clone();
                cleared.base = None;
                (cleared, Some(base_ref), Some(reference))
            }
        }
    }

    fn build_env(&mut self, parent: &Env, props: &Map<String, Value>) -> Env {
        let mut env = parent.clone();
        apply_declarations(&mut env, props);
        self.apply_variables_blocks(props.get("variables"), &mut env);
        env
    }

    fn apply_variables_blocks(&mut self, blocks: Option<&Value>, env: &mut Env) {
        let Some(Value::Array(blocks)) = blocks else {
            return;
        };
        let mut sink = Vec::new();
        for block in blocks {
            let Value::Object(entries) = block else {
                continue;
            };
            let selected = match entries.get("requires").and_then(Value::as_str) {
                Some(expression) => predicate::eval(expression, env) == Some(true),
                None => true,
            };
            if !selected {
                continue;
            }
            for (key, value) in entries {
                if key == "requires" {
                    continue;
                }
                if let Some((name, _)) = parse_var_key(key) {
                    let resolved = fold_expression(value, substitute(value, env, &mut sink), env);
                    env.set(name, resolved);
                }
            }
        }
    }

    fn extract_factory(
        &mut self,
        control: &RawControl,
        control_type: Option<&str>,
        env: &Env,
    ) -> (Option<Factory>, bool) {
        let owner = &control.owner_ns;
        if let Some(Value::Object(spec)) = control.props.get("factory") {
            let name = spec.get("name").and_then(Value::as_str).map(str::to_owned);
            let control_ids = control_id_map(spec.get("control_ids"), owner, env);
            let control_name = spec
                .get("control_name")
                .and_then(Value::as_str)
                .map(|reference| parse_reference(reference, owner, env));
            let factory = Factory {
                name,
                control_ids,
                control_name,
            };
            if !factory.is_empty() || factory.name.is_some() {
                return (Some(factory), false);
            }
        }
        if control_type == Some("factory") {
            let control_ids = control_id_map(control.props.get("control_ids"), owner, env);
            if !control_ids.is_empty() {
                return (
                    Some(Factory {
                        name: None,
                        control_ids,
                        control_name: None,
                    }),
                    true,
                );
            }
        }
        (None, false)
    }
}

fn build_properties(
    control: &RawControl,
    env: &Env,
    control_ids_consumed: bool,
    missing: &mut Vec<String>,
) -> std::collections::BTreeMap<String, Value> {
    let mut properties = std::collections::BTreeMap::new();
    for (key, value) in &control.props {
        if key.starts_with('$') || is_reserved(key) {
            continue;
        }
        if control_ids_consumed && key == "control_ids" {
            continue;
        }
        let substituted = substitute(value, env, missing);
        properties.insert(key.clone(), fold_expression(value, substituted, env));
    }
    properties
}

fn is_reserved(key: &str) -> bool {
    matches!(key, "type" | "ignored" | "variables" | "factory")
}

fn control_id_map(
    value: Option<&Value>,
    owner: &str,
    env: &Env,
) -> std::collections::BTreeMap<String, ControlRef> {
    let mut map = std::collections::BTreeMap::new();
    if let Some(Value::Object(entries)) = value {
        for (role, reference) in entries {
            if let Some(text) = reference.as_str() {
                map.insert(role.clone(), parse_reference(text, owner, env));
            }
        }
    }
    map
}

fn parse_reference(reference: &str, owner: &str, env: &Env) -> ControlRef {
    let resolved = match reference.strip_prefix('$') {
        Some(variable) => match env.get(variable) {
            Some(Value::String(text)) => text.clone(),
            _ => reference.to_owned(),
        },
        None => reference.to_owned(),
    };
    ControlRef::parse(&resolved, owner)
}

fn value_string(value: Value) -> Option<String> {
    match value {
        Value::String(text) => Some(text),
        _ => None,
    }
}
