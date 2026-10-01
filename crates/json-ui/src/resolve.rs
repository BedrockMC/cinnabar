//! Stage C: turn raw controls into the resolved tree. For each control we build a
//! variable scope (inherited scope, then `$decl`s, then matching `variables[]`
//! blocks), substitute `$vars` in properties, record any factory, drop `ignored`
//! children, and recurse. Base references on child keys are resolved here because
//! they may be `$var`s that only the scope knows.

use serde_json::{Map, Value};

use crate::anim;
use crate::catalog::{Catalog, RawControl, child_controls};
use crate::env::{Env, apply_declarations, fold_expression, parse_var_key, substitute};
use crate::merge::{deep_merge_control, flatten_def};
use crate::predicate;
use crate::tree::{ControlRef, Factory, ResolvedControl};

const MAX_DEPTH: usize = 256;
/// Property recording the `$vars` a factory's or grid's created controls see.
pub(crate) const FACTORY_SCOPE: &str = "factory_scope";
/// A digest of [`FACTORY_SCOPE`], so equal scopes are recognised without comparing.
pub(crate) const FACTORY_SCOPE_KEY: &str = "factory_scope_key";
const MAX_NODES: usize = 200_000;

/// Drives resolution over one [`Catalog`], accumulating diagnostics.
pub struct Resolver<'a> {
    catalog: &'a Catalog,
    diagnostics: Vec<String>,
    nodes: usize,
    /// The scope resolution started in; a factory records what it adds to it.
    root: Option<Env>,
}

impl<'a> Resolver<'a> {
    pub fn new(catalog: &'a Catalog) -> Self {
        Self {
            catalog,
            diagnostics: Vec::new(),
            nodes: 0,
            root: None,
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
        if self.root.is_none() {
            self.root = Some(root_env.clone());
        }
        let (control, provenance) =
            flatten_def(self.catalog, namespace, name, &mut self.diagnostics)?;
        let env = self.build_env(root_env, &control.props);
        // An ignored definition creates nothing, whether a screen or a
        // factory's instance (a pack's title overlay gated on one title).
        if self.is_ignored(&control, &env) {
            return None;
        }
        Some(self.resolve_with_env(&control, provenance, None, &env, 0))
    }

    /// The root's `type` and substituted properties without resolving its
    /// children; `None` when the reference is unknown or ignored.
    pub(crate) fn resolve_root_properties(
        &mut self,
        namespace: &str,
        name: &str,
        root_env: &Env,
    ) -> Option<(Option<String>, std::collections::BTreeMap<String, Value>)> {
        let (control, _) = flatten_def(self.catalog, namespace, name, &mut self.diagnostics)?;
        let env = self.build_env(root_env, &control.props);
        if self.is_ignored(&control, &env) {
            return None;
        }
        let mut missing = Vec::new();
        let control_type = control
            .props
            .get("type")
            .map(|value| substitute(value, &env, &mut missing))
            .and_then(value_string);
        let properties = build_properties(&control, &env, false, &mut missing);
        Some((control_type, properties))
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
        let mut properties = build_properties(control, env, control_ids_consumed, &mut missing);
        self.resolve_anims(&mut properties, env);
        if factory.is_some() || properties.contains_key("grid_item_template") {
            let scope = self.local_scope(env);
            let key = scope_key(&scope);
            properties.insert(FACTORY_SCOPE.to_owned(), scope);
            properties.insert(FACTORY_SCOPE_KEY.to_owned(), Value::String(key));
        }
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
            name: instance_name(&control.name, env),
            control_type,
            base: provenance,
            unresolved_base,
            properties,
            children,
            factory,
        }
    }

    /// The variables `env` holds beyond the root scope, which the controls a
    /// factory or grid creates resolve with, as they would inside it.
    fn local_scope(&self, env: &Env) -> Value {
        let root = self.root.as_ref();
        Value::Object(
            env.iter()
                .filter(|(name, value)| root.and_then(|root| root.get(name)) != Some(*value))
                .map(|(name, value)| (name.clone(), value.clone()))
                .collect(),
        )
    }

    /// Replace `@anim` references in `alpha`/`offset`/`anims` with their resolved chains,
    /// and an animated `uv` with its first frame plus the flip-book that plays it.
    fn resolve_anims(&self, properties: &mut std::collections::BTreeMap<String, Value>, env: &Env) {
        if let Some(Value::String(reference)) = properties.get("uv")
            && reference.starts_with('@')
        {
            match anim::resolve_flip_book(self.catalog, reference, env) {
                Some(book) => {
                    properties.insert("uv".to_owned(), serde_json::json!(book.initial_uv));
                    if let Ok(value) = serde_json::to_value(book) {
                        properties.insert(anim::FLIP_BOOK_KEY.to_owned(), value);
                    }
                }
                None => {
                    properties.remove("uv");
                }
            }
        }
        let mut chains = Vec::new();
        if let Some(Value::String(reference)) = properties.get("alpha")
            && reference.starts_with('@')
        {
            chains.extend(anim::resolve_chain(self.catalog, reference, env));
            properties.remove("alpha");
        }
        let mut slide = None;
        if let Some(Value::String(reference)) = properties.get("offset")
            && reference.starts_with('@')
        {
            slide = anim::resolve_slide(self.catalog, reference, env);
            properties.remove("offset");
        }
        if let Some(Value::Array(items)) = properties.get("anims") {
            for reference in items.iter().filter_map(Value::as_str) {
                if reference.starts_with('@') {
                    chains.extend(anim::resolve_chain(self.catalog, reference, env));
                    if slide.is_none() {
                        slide = anim::resolve_slide(self.catalog, reference, env);
                    }
                }
            }
        }
        if !chains.is_empty()
            && let Ok(value) = serde_json::to_value(chains)
        {
            properties.insert(anim::CHAINS_KEY.to_owned(), value);
        }
        if let Some(value) = slide.and_then(|slide| serde_json::to_value(slide).ok()) {
            properties.insert(anim::SLIDE_KEY.to_owned(), value);
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
        // A `$var` child list (`"controls": "$button_contents"`) is read here.
        let dynamic = match control.props.get("controls") {
            Some(Value::String(reference)) => {
                match reference.strip_prefix('$').and_then(|name| env.get(name)) {
                    Some(value) => child_controls(&control.owner_ns, value, &mut self.diagnostics),
                    None => Vec::new(),
                }
            }
            _ => Vec::new(),
        };
        let children = if dynamic.is_empty() {
            &control.children
        } else {
            &dynamic
        };
        let mut resolved = Vec::new();
        for child in children {
            let (working, provenance, unresolved) = self.resolve_child_base(child, env);
            // `ignored` reads the enclosing scope only: the vanilla client
            // evaluates it before the control's own `$` declarations apply.
            if self.is_ignored(&working, env) {
                continue;
            }
            let child_env = self.build_env(env, &working.props);
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
                        "{}.{}: undecidable `ignored` `{}` ({} bytes); keeping",
                        control.owner_ns,
                        control.name,
                        clipped(expression),
                        expression.len()
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
        // `{ "$button_layout": {} }` with `$button_layout: "@ns.panel"` instances
        // that panel, named as it is (the disconnect screen's buttons).
        if child.base.is_none()
            && let Some(Value::String(text)) = child
                .name
                .strip_prefix('$')
                .and_then(|variable| env.get(variable))
            && let Some(reference) = text.strip_prefix('@')
        {
            let mut named = child.clone();
            named.name = ControlRef::parse(reference, &child.owner_ns).name;
            named.base = Some(reference.to_owned());
            return self.resolve_child_base(&named, env);
        }
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
        match flatten_def(
            self.catalog,
            &base_ref.namespace,
            &base_ref.name,
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
            let max_children_size = spec
                .get("max_children_size")
                .and_then(Value::as_u64)
                .map(|max| max as usize);
            let factory = Factory {
                name,
                control_ids,
                control_name,
                max_children_size,
            };
            if !factory.is_empty() || factory.name.is_some() {
                return (Some(factory), false);
            }
        }
        // A `type: "factory"` control is its own factory, named by its instance.
        if control_type == Some("factory") {
            let control_ids = control_id_map(control.props.get("control_ids"), owner, env);
            let control_name = control
                .props
                .get("control_name")
                .and_then(Value::as_str)
                .map(|reference| parse_reference(reference, owner, env));
            if !control_ids.is_empty() || control_name.is_some() {
                return (
                    Some(Factory {
                        name: Some(instance_name(&control.name, env)),
                        control_ids,
                        control_name,
                        max_children_size: control
                            .props
                            .get("max_children_size")
                            .and_then(Value::as_u64)
                            .map(|max| max as usize),
                    }),
                    true,
                );
            }
        }
        (None, false)
    }
}

/// A stable digest of a scope's serialized vars.
fn scope_key(scope: &Value) -> String {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    serde_json::to_string(scope)
        .unwrap_or_default()
        .hash(&mut hasher);
    format!("{:016x}", hasher.finish())
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
    matches!(
        key,
        "type" | "ignored" | "variables" | "factory" | "controls"
    )
}

/// A factory's role map; the whole map may itself be a `$var` holding an object.
fn control_id_map(
    value: Option<&Value>,
    owner: &str,
    env: &Env,
) -> std::collections::BTreeMap<String, ControlRef> {
    let mut map = std::collections::BTreeMap::new();
    let value = value.map(|value| substitute(value, env, &mut Vec::new()));
    if let Some(Value::Object(entries)) = &value {
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

/// A diagnostic-sized prefix of server-supplied text.
fn clipped(text: &str) -> &str {
    let mut end = text.len().min(80);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}

fn value_string(value: Value) -> Option<String> {
    match value {
        Value::String(text) => Some(text),
        _ => None,
    }
}

/// An instance name written as a `$var` (`"$tab_view_binding_name@common.toggle"`)
/// takes the variable's string value; view bindings find the control by it.
fn instance_name(name: &str, env: &Env) -> String {
    match name
        .strip_prefix('$')
        .and_then(|variable| env.get(variable))
    {
        Some(Value::String(value)) => value.clone(),
        _ => name.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use crate::{Catalog, Context, resolve};

    // A control's own `$` declarations do not decide its `ignored`.
    #[test]
    fn ignored_reads_the_enclosing_scope() {
        let mut catalog = Catalog::default();
        catalog.overlay_text(
            "ui/a.json",
            r#"{ "namespace": "n", "root": { "type": "panel", "$touch_mode|default": false,
                "controls": [
                    { "touch": { "type": "panel", "ignored": "(not $touch_mode)", "$touch_mode": true } },
                    { "mouse": { "type": "panel", "ignored": "$touch_mode" } } ] } }"#,
        );
        let root = resolve(&catalog, "n.root", &Context::empty())
            .control
            .unwrap();
        let names: Vec<_> = root
            .children
            .iter()
            .map(|child| child.name.as_str())
            .collect();
        assert_eq!(names, ["mouse"]);
    }
}
