//! Variable environment and `$var` substitution. Keys are stored without the
//! leading `$`. Substitution replaces `$name` tokens in property values; it never
//! evaluates size/`view` arithmetic, so an expression like `"100% - 15px"` is
//! copied verbatim.

use std::{collections::BTreeMap, sync::Arc};

use serde_json::{Map, Value};

/// A flat variable scope. Descending the tree shares the parent values until a
/// control declares a variable, so inner definitions shadow outer ones.
#[derive(Clone, Debug, Default)]
pub struct Env {
    vars: Arc<BTreeMap<String, Value>>,
}

impl Env {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn get(&self, name: &str) -> Option<&Value> {
        self.vars.get(name)
    }

    pub fn contains(&self, name: &str) -> bool {
        self.vars.contains_key(name)
    }

    pub fn set(&mut self, name: impl Into<String>, value: Value) {
        let name = name.into();
        if !self
            .vars
            .get(&name)
            .is_some_and(|old| same_representation(old, &value))
        {
            Arc::make_mut(&mut self.vars).insert(name, value);
        }
    }

    pub fn iter(&self) -> impl Iterator<Item = (&String, &Value)> {
        self.vars.iter()
    }
}

/// Compare values as substitution will spell them, including signed zero and object order.
fn same_representation(left: &Value, right: &Value) -> bool {
    match (left, right) {
        (Value::Number(a), Value::Number(b)) => {
            a == b && a.as_f64().map(f64::to_bits) == b.as_f64().map(f64::to_bits)
        }
        (Value::Array(a), Value::Array(b)) => {
            a.len() == b.len() && a.iter().zip(b).all(|(a, b)| same_representation(a, b))
        }
        (Value::Object(a), Value::Object(b)) => {
            a.len() == b.len()
                && a.iter()
                    .zip(b)
                    .all(|((ak, av), (bk, bv))| ak == bk && same_representation(av, bv))
        }
        _ => left == right,
    }
}

/// Apply a control's `$decl` properties onto `env`. A `$x|default` fills `x` only
/// when it is otherwise unset (inherited or concrete definitions win); a plain
/// `$x` always overrides. Values are substituted as they are applied, so a
/// declaration may reference variables already in scope.
pub fn apply_declarations(env: &mut Env, props: &Map<String, Value>) {
    let mut sink = Vec::new();
    let mut concretes = Vec::new();
    for (key, value) in props {
        let Some((name, is_default)) = parse_var_key(key) else {
            continue;
        };
        if is_default {
            if !env.contains(&name) {
                let resolved = fold_expression(value, substitute(value, env, &mut sink), env);
                env.set(name, resolved);
            }
        } else {
            concretes.push((name, value));
        }
    }
    for (name, value) in concretes {
        let resolved = fold_expression(value, substitute(value, env, &mut sink), env);
        env.set(name, resolved);
    }
}

/// Evaluate a parenthesised string expression built from `$vars` (e.g.
/// `('#' + $dropdown_name)`) once its variables are substituted. Only raw values
/// that start with `(` and reference a `$var` qualify, so literal text in
/// parentheses survives; an expression that still needs runtime `#bindings` is
/// left for the binder.
pub fn fold_expression(raw: &Value, substituted: Value, env: &Env) -> Value {
    let Value::String(raw) = raw else {
        return substituted;
    };
    if !raw.trim_start().starts_with('(') || !raw.contains('$') {
        return substituted;
    }
    let Value::String(expression) = &substituted else {
        return substituted;
    };
    match crate::predicate::eval_scalar(expression, env, &crate::predicate::NoBindings) {
        Some(crate::predicate::Scalar::Bool(flag)) => Value::Bool(flag),
        Some(crate::predicate::Scalar::Text(text)) => Value::String(text),
        Some(crate::predicate::Scalar::Num(number)) => serde_json::Number::from_f64(number)
            .map(Value::Number)
            .unwrap_or(substituted),
        None => substituted,
    }
}

/// Strip a leading `$` and, for a declaration key, the `|default` suffix.
/// Returns `(name, is_default)`; a non-`$` key yields `None`.
pub fn parse_var_key(key: &str) -> Option<(String, bool)> {
    let rest = key.strip_prefix('$')?;
    match rest.split_once('|') {
        Some((name, _modifier)) => Some((name.to_owned(), true)),
        None => Some((rest.to_owned(), false)),
    }
}

/// Substitute `$var` references throughout a value using `env`. Unknown variables
/// are left in place and reported through `unresolved`.
pub fn substitute(value: &Value, env: &Env, unresolved: &mut Vec<String>) -> Value {
    substitute_within(value, env, unresolved, 0)
}

/// How many times a variable's value may itself name a variable.
const MAX_SUBSTITUTION_DEPTH: usize = 8;

fn substitute_within(
    value: &Value,
    env: &Env,
    unresolved: &mut Vec<String>,
    depth: usize,
) -> Value {
    match value {
        Value::String(text) => substitute_string(text, env, unresolved, depth),
        Value::Array(items) => Value::Array(
            items
                .iter()
                .map(|item| substitute_within(item, env, unresolved, depth))
                .collect(),
        ),
        Value::Object(map) => Value::Object(
            map.iter()
                .map(|(key, item)| (key.clone(), substitute_within(item, env, unresolved, depth)))
                .collect(),
        ),
        other => other.clone(),
    }
}

fn substitute_string(text: &str, env: &Env, unresolved: &mut Vec<String>, depth: usize) -> Value {
    let Some(first) = text.find('$') else {
        return Value::String(text.to_owned());
    };
    // Exact single-token form (`"$var"`): preserve the referenced value's type.
    if first == 0
        && let Some(name) = whole_token(text)
    {
        // The value may carry further `$vars` (a shared binding list naming
        // `$condition`), replaced in turn as the vanilla client does.
        return match env.get(&name) {
            Some(value) if depth < MAX_SUBSTITUTION_DEPTH => {
                substitute_within(value, env, unresolved, depth + 1)
            }
            Some(value) => value.clone(),
            None => {
                unresolved.push(name);
                Value::String(text.to_owned())
            }
        };
    }
    Value::String(replace_tokens(text, env, unresolved))
}

/// The full string is one `$name` token, or `None` if there is trailing text.
fn whole_token(text: &str) -> Option<String> {
    let name = &text[1..];
    if !name.is_empty() && name.bytes().all(is_ident_byte) {
        Some(name.to_owned())
    } else {
        None
    }
}

fn replace_tokens(text: &str, env: &Env, unresolved: &mut Vec<String>) -> String {
    // Inside a parenthesised expression a string variable is one string operand,
    // as the vanilla client makes a token from the variable's value; a value
    // naming a `#binding` stays that binding.
    let expression = text.trim_start().starts_with('(');
    let bytes = text.as_bytes();
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'$' {
            let start = i + 1;
            let mut end = start;
            while end < bytes.len() && is_ident_byte(bytes[end]) {
                end += 1;
            }
            if end > start {
                let name = &text[start..end];
                match env.get(name) {
                    Some(Value::String(value))
                        if expression && !value.contains('\'') && !value.starts_with('#') =>
                    {
                        out.push('\'');
                        out.push_str(value);
                        out.push('\'');
                    }
                    Some(value) => out.push_str(&scalar_string(value).unwrap_or_default()),
                    None => {
                        unresolved.push(name.to_owned());
                        out.push_str(&text[i..end]);
                    }
                }
                i = end;
                continue;
            }
        }
        // Advance by one full UTF-8 char to keep the output well-formed.
        let ch = text[i..].chars().next().unwrap();
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}

fn scalar_string(value: &Value) -> Option<String> {
    match value {
        Value::String(text) => Some(text.clone()),
        Value::Number(number) => Some(number.to_string()),
        Value::Bool(flag) => Some(flag.to_string()),
        _ => None,
    }
}

fn is_ident_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_'
}

#[cfg(test)]
mod tests {
    use super::{Env, apply_declarations, fold_expression, parse_var_key, substitute};
    use serde_json::{Map, json};

    #[test]
    fn inherited_scopes_share_values_until_a_declaration_changes_them() {
        let parent = env();
        let mut child = parent.clone();
        let sibling = parent.clone();
        for _ in 0..100 {
            let descendant = child.clone();
            assert!(std::sync::Arc::ptr_eq(&parent.vars, &descendant.vars));
        }
        child.set("name", parent.get("name").unwrap().clone());
        assert!(std::sync::Arc::ptr_eq(&parent.vars, &child.vars));
        apply_declarations(&mut child, &props(json!({"$name": "child"})));
        assert!(!std::sync::Arc::ptr_eq(&parent.vars, &child.vars));
        assert!(std::sync::Arc::ptr_eq(&parent.vars, &sibling.vars));
        assert_eq!(parent.get("name"), Some(&json!("#title_text")));
        assert_eq!(sibling.get("name"), parent.get("name"));
        assert_eq!(child.get("name"), Some(&json!("child")));
        assert_eq!(child.get("title_size"), parent.get("title_size"));
    }

    #[test]
    fn equal_numbers_with_different_signed_zero_representations_replace_the_scope_value() {
        for (old, new) in [
            (json!(0.0), json!(-0.0)),
            (json!([0.0]), json!([-0.0])),
            (json!({"n": [0.0]}), json!({"n": [-0.0]})),
        ] {
            let mut parent = Env::new();
            parent.set("n", old.clone());
            let mut child = parent.clone();
            child.set("n", new.clone());
            assert_eq!(child.get("n").unwrap().to_string(), new.to_string());
            assert_eq!(parent.get("n").unwrap().to_string(), old.to_string());
            assert!(!std::sync::Arc::ptr_eq(&parent.vars, &child.vars));
        }
        let mut scope = Env::new();
        scope.set("n", json!(0.0));
        scope.set("n", json!(-0.0));
        let mut unknown = Vec::new();
        assert_eq!(
            substitute(&json!("value: $n"), &scope, &mut unknown),
            json!("value: -0.0")
        );
    }

    #[test]
    #[ignore = "benchmark"]
    fn frame_cost_bench_inherited_variable_scopes() {
        let mut parent = Env::new();
        for index in 0..500 {
            parent.set(
                format!("variable_{index}"),
                json!([index, "inherited pack value"]),
            );
        }
        const SCOPES: u32 = 1_000;
        let started = std::time::Instant::now();
        for _ in 0..SCOPES {
            std::hint::black_box((*parent.vars).clone());
        }
        let old = started.elapsed();
        let started = std::time::Instant::now();
        for _ in 0..SCOPES {
            std::hint::black_box(parent.clone());
        }
        let new = started.elapsed();
        eprintln!(
            "FRAME_COST inherited_variable_scopes_1000x500: old={:.3}ms new={:.3}ms",
            old.as_secs_f64() * 1e3,
            new.as_secs_f64() * 1e3
        );
    }

    fn props(value: serde_json::Value) -> Map<String, serde_json::Value> {
        match value {
            serde_json::Value::Object(map) => map,
            _ => unreachable!(),
        }
    }

    #[test]
    fn default_fills_only_when_absent_concrete_always_overrides() {
        let mut env = Env::new();
        env.set("provided", json!("outer"));
        apply_declarations(
            &mut env,
            &props(json!({
                "$provided|default": "fallback",
                "$fresh|default": "made",
                "$explicit": "set",
            })),
        );
        assert_eq!(env.get("provided"), Some(&json!("outer")));
        assert_eq!(env.get("fresh"), Some(&json!("made")));
        assert_eq!(env.get("explicit"), Some(&json!("set")));
    }

    #[test]
    fn concrete_declaration_resolves_against_scope() {
        let mut env = Env::new();
        env.set("custom_background", json!("dialog_background_hollow_3"));
        apply_declarations(
            &mut env,
            &props(json!({ "$dialog_background": "$custom_background" })),
        );
        assert_eq!(
            env.get("dialog_background"),
            Some(&json!("dialog_background_hollow_3"))
        );
    }

    fn env() -> Env {
        let mut env = Env::new();
        env.set("title_size", json!(["100% - 15px", 10]));
        env.set("name", json!("#title_text"));
        env
    }

    #[test]
    fn exact_reference_preserves_array_type() {
        let out = substitute(&json!("$title_size"), &env(), &mut Vec::new());
        assert_eq!(out, json!(["100% - 15px", 10]));
    }

    #[test]
    fn arithmetic_strings_are_left_symbolic() {
        let out = substitute(&json!("100% - 15px"), &env(), &mut Vec::new());
        assert_eq!(out, json!("100% - 15px"));
    }

    #[test]
    fn unknown_variable_is_reported_and_kept() {
        let mut missing = Vec::new();
        let out = substitute(&json!("$nope"), &env(), &mut missing);
        assert_eq!(out, json!("$nope"));
        assert_eq!(missing, vec!["nope".to_owned()]);
    }

    #[test]
    fn parenthesised_var_expressions_fold_but_literals_survive() {
        let mut env = Env::new();
        env.set("dropdown_name", json!("custom_dropdown"));
        let raw = json!("('#' + $dropdown_name)");
        let folded = fold_expression(&raw, substitute(&raw, &env, &mut Vec::new()), &env);
        assert_eq!(folded, json!("#custom_dropdown"));
        let literal = json!("(Beta)");
        assert_eq!(
            fold_expression(&literal, literal.clone(), &env),
            json!("(Beta)")
        );
        let runtime = json!("(not #enabled)");
        assert_eq!(fold_expression(&runtime, runtime.clone(), &env), runtime);
    }

    // Marker strings such as `§j` or `@pack/form` stay single operands.
    #[test]
    fn string_variables_substitute_into_expressions_as_literals() {
        let mut env = Env::new();
        env.set("boxes", json!("@mineville/boxes"));
        let out = substitute(&json!("(not ((#t - $boxes) = #t))"), &env, &mut Vec::new());
        assert_eq!(out, json!("(not ((#t - '@mineville/boxes') = #t))"));
        assert_eq!(
            substitute(&json!("a $boxes"), &env, &mut Vec::new()),
            json!("a @mineville/boxes")
        );
    }

    // `(not $cell_selected_binding_name)` names the binding the variable holds.
    #[test]
    fn a_variable_holding_a_binding_name_stays_a_binding_in_expressions() {
        let mut env = Env::new();
        env.set("selected", json!("#is_selected_slot"));
        let out = substitute(&json!("(not $selected)"), &env, &mut Vec::new());
        assert_eq!(out, json!("(not #is_selected_slot)"));
    }

    #[test]
    fn a_substituted_value_has_its_own_variables_replaced() {
        let mut env = Env::new();
        env.set(
            "visible_binding",
            json!([{ "binding_type": "view", "source_property_name": "$condition" }]),
        );
        env.set("condition", json!("(#a = '')"));
        let out = substitute(&json!("$visible_binding"), &env, &mut Vec::new());
        assert_eq!(out[0]["source_property_name"], json!("(#a = '')"));
    }

    #[test]
    fn default_declaration_key_is_recognised() {
        assert_eq!(parse_var_key("$x|default"), Some(("x".to_owned(), true)));
        assert_eq!(parse_var_key("$x"), Some(("x".to_owned(), false)));
        assert_eq!(parse_var_key("size"), None);
    }
}
