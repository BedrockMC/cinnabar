//! Variable scopes, conditionals and factory declarations resolve as the vanilla
//! client resolves them; each fixture is one audited behaviour.

use json_ui::{Catalog, Context, ResolvedControl};
use serde_json::{Value, json};

fn catalog(body: &str) -> Catalog {
    let mut catalog = Catalog::default();
    catalog.overlay_text("ui/a.json", &format!(r#"{{"namespace":"a",{body}}}"#));
    catalog
}

fn resolve_in(body: &str, context: &Context) -> Option<ResolvedControl> {
    json_ui::resolve(&catalog(body), "a.root", context).control
}

fn root(body: &str) -> ResolvedControl {
    resolve_in(body, &Context::empty()).expect("root resolves")
}

fn text(control: &ResolvedControl) -> Value {
    control.properties.get("text").cloned().unwrap_or(Value::Null)
}

fn root_text(control: &str) -> Value {
    text(&root(&format!(r#""root":{control}"#)))
}

// A nearer `|default` beats an outer one; a block's default never beats a concrete value.
#[test]
fn defaults_resolve_after_every_concrete_scope() {
    let nearer = root(
        r#""root":{"type":"panel","$x|default":"outer","controls":[
            {"child":{"type":"label","$x|default":"inner","text":"$x"}}]}"#,
    );
    assert_eq!(text(&nearer.children[0]), json!("inner"));
    let concrete = root(
        r#""root":{"type":"panel","$x":"concrete","controls":[
            {"child":{"type":"label","variables":[{"requires":"true","$x|default":"fallback"}],
              "text":"$x"}}]}"#,
    );
    assert_eq!(text(&concrete.children[0]), json!("concrete"));
}

// Names are exact: dotted names and direct `|default` references resolve, and only
// the exact `|default` suffix has fallback meaning.
#[test]
fn variable_names_are_exact() {
    assert_eq!(root_text(r#"{"type":"label","$a.b":"OK","text":"$a.b"}"#), json!("OK"));
    assert_eq!(
        root_text(r#"{"type":"label","$x|default":"OK","text":"$x|default"}"#),
        json!("OK")
    );
    assert_eq!(
        root_text(r#"{"type":"label","$x|weird":"OK","text":"$x"}"#),
        json!("$x")
    );
}

// A structured variable keeps its nested strings for the consumer's own scope.
#[test]
fn structured_variables_resolve_in_their_consumers_scope() {
    let context = Context::empty().with_var("x", json!("outer"));
    let tree = resolve_in(
        r#""root":{"type":"panel","$children":[{"child":{"type":"label","$x":"inner","text":"$x"}}],
            "controls":"$children"}"#,
        &context,
    )
    .unwrap();
    assert_eq!(text(&tree.children[0]), json!("inner"));
}

// Only a whole-string `$name` is a reference; quoted operands stay literal.
#[test]
fn strings_are_not_interpolation_templates() {
    assert_eq!(
        root_text(r#"{"type":"label","$x":"red","text":"Color: $x"}"#),
        json!("Color: $x")
    );
    assert_eq!(root_text(r#"{"type":"label","$x":"red","text":"('$x')"}"#), json!("$x"));
    assert_eq!(
        root_text(r#"{"type":"label","$x":"Joe's","text":"('Hi ' + $x)"}"#),
        json!("Hi Joe's")
    );
}

// `__string` wrappers yield their `value`, raw text without evaluating it.
#[test]
fn wrapped_string_variables_unwrap() {
    assert_eq!(
        root_text(
            r#"{"type":"label","$s":{"__string":true,"__rawtext":true,"value":"(1 + 1)"},"text":"$s"}"#
        ),
        json!("(1 + 1)")
    );
    assert_eq!(
        root_text(r#"{"type":"label","$s":{"__string":true,"value":"(1 + 1)"},"text":"$s"}"#),
        json!(2)
    );
}

// A constant parenthesised property evaluates without any `$` reference.
#[test]
fn constant_property_expressions_fold() {
    let tree = root(r#""root":{"type":"panel","alpha":"(1.0 / 2.0)"}"#);
    assert_eq!(tree.properties["alpha"], json!(0.5));
}

// `variables` may be one object or a `$var` holding the blocks.
#[test]
fn variables_accept_objects_and_variables() {
    assert_eq!(
        root_text(r#"{"type":"label","$x":"old","variables":{"requires":"true","$x":"new"},"text":"$x"}"#),
        json!("new")
    );
    assert_eq!(
        root_text(
            r#"{"type":"label","$x":"old","$blocks":[{"requires":"true","$x":"new"}],"variables":"$blocks","text":"$x"}"#
        ),
        json!("new")
    );
}

// `requires` selects by type: nonzero numbers and nonempty strings; missing never.
#[test]
fn requires_uses_the_typed_dispatch() {
    let with = |condition: &str| {
        root_text(&format!(
            r#"{{"type":"label","$x":"old","variables":[{{"requires":{condition},"$x":"new"}}],"text":"$x"}}"#
        ))
    };
    for condition in ["false", "0", "null", "[]", r#""""#] {
        assert_eq!(with(condition), json!("old"), "{condition}");
    }
    for condition in [r#""false""#, r#""(1)""#, "true", "2"] {
        assert_eq!(with(condition), json!("new"), "{condition}");
    }
    assert_eq!(
        root_text(r#"{"type":"label","$x":"old","variables":[{"$x":"new"}],"text":"$x"}"#),
        json!("old")
    );
}

// `ignored` acts on integers and bools, not on literal strings, and reads the
// enclosing scope even at the root.
#[test]
fn ignored_uses_the_typed_dispatch_and_enclosing_scope() {
    assert!(resolve_in(r#""root":{"type":"panel","ignored":1}"#, &Context::empty()).is_none());
    assert!(resolve_in(r#""root":{"type":"panel","ignored":"(1)"}"#, &Context::empty()).is_none());
    assert!(resolve_in(r#""root":{"type":"panel","ignored":"true"}"#, &Context::empty()).is_some());
    let context = Context::empty().with_flag("omit", false);
    assert!(resolve_in(r#""root":{"type":"panel","$omit":true,"ignored":"$omit"}"#, &context).is_some());
}
