//! Data-binding unit tests over hand-built control trees: global/collection/
//! collection_details/view resolution, `#collection_length`, and factory expansion.
//! These need no `.local` pack; a stub [`ControlLibrary`] supplies factory controls.

use std::collections::BTreeMap;

use json_ui::{
    CollectionItem, ControlLibrary, ControlRef, DataSource, EmptyLibrary, ResolvedControl, Scalar,
    bind,
};
use serde_json::{Value, json};

fn ctrl(name: &str, control_type: Option<&str>, props: Value) -> ResolvedControl {
    ctrl_children(name, control_type, props, Vec::new())
}

fn ctrl_children(
    name: &str,
    control_type: Option<&str>,
    props: Value,
    children: Vec<ResolvedControl>,
) -> ResolvedControl {
    let properties: BTreeMap<String, Value> = match props {
        Value::Object(map) => map.into_iter().collect(),
        _ => BTreeMap::new(),
    };
    ResolvedControl {
        name: name.to_owned(),
        control_type: control_type.map(str::to_owned),
        base: None,
        unresolved_base: None,
        properties,
        children,
        factory: None,
    }
}

fn factory_panel(
    name: &str,
    collection: &str,
    control_ids: &[(&str, ControlRef)],
) -> ResolvedControl {
    let mut control = ctrl(
        name,
        Some("stack_panel"),
        json!({ "collection_name": collection }),
    );
    control.factory = Some(json_ui::Factory {
        name: Some("buttons".to_owned()),
        control_ids: control_ids
            .iter()
            .map(|(role, reference)| ((*role).to_owned(), reference.clone()))
            .collect(),
        control_name: None,
    });
    control
}

fn prop<'a>(control: &'a ResolvedControl, key: &str) -> &'a Value {
    control
        .properties
        .get(key)
        .unwrap_or_else(|| panic!("{} missing {key}", control.name))
}

/// A [`ControlLibrary`] backed by a fixed map of references to control trees.
struct StubLibrary(BTreeMap<String, ResolvedControl>);

impl ControlLibrary for StubLibrary {
    fn resolve(&self, reference: &ControlRef) -> Option<ResolvedControl> {
        self.0
            .get(&format!("{}.{}", reference.namespace, reference.name))
            .cloned()
    }
}

#[test]
fn global_binding_bakes_into_text() {
    let label = ctrl(
        "title",
        Some("label"),
        json!({
            "text": "#title_text",
            "bindings": [ { "binding_name": "#title_text" } ],
        }),
    );
    let mut data = DataSource::new();
    data.set_global("#title_text", Scalar::Text("Welcome".into()));

    let bound = bind(&label, &data, &EmptyLibrary);
    assert_eq!(prop(&bound, "text"), &json!("Welcome"));
}

#[test]
fn global_binding_override_targets_a_different_name() {
    // `#submit_button_visible` drives `#visible`, exactly the submit-button binding.
    let button = ctrl(
        "submit",
        Some("button"),
        json!({
            "bindings": [
                { "binding_name": "#submit_button_visible", "binding_name_override": "#visible" }
            ],
        }),
    );
    let mut data = DataSource::new();
    data.set_global("#submit_button_visible", Scalar::Bool(false));

    let bound = bind(&button, &data, &EmptyLibrary);
    assert_eq!(prop(&bound, "visible"), &json!(false));
}

#[test]
fn collection_length_binding_exposes_the_count() {
    let label = ctrl(
        "counter",
        Some("label"),
        json!({
            "text": "#collection_length",
            "bindings": [
                { "binding_name": "#form_button_contents", "binding_name_override": "#collection_length" }
            ],
        }),
    );
    let mut data = DataSource::new();
    data.set_global("#form_button_contents", Scalar::Num(3.0));

    let bound = bind(&label, &data, &EmptyLibrary);
    assert_eq!(prop(&bound, "text").as_f64(), Some(3.0));
}

#[test]
fn factory_instantiates_one_control_per_collection_index() {
    let row = ctrl(
        "row",
        Some("label"),
        json!({
            "text": "#cell",
            "bindings": [
                { "binding_type": "collection", "binding_collection_name": "items", "binding_name": "#cell" }
            ],
        }),
    );
    let lib = StubLibrary([("ns.row".to_owned(), row)].into_iter().collect());
    let panel = factory_panel("list", "items", &[("button", ControlRef::new("ns", "row"))]);

    let mut data = DataSource::new();
    data.set_collection(
        "items",
        vec![
            CollectionItem::new("button").with("#cell", Scalar::Text("A".into())),
            CollectionItem::new("button").with("#cell", Scalar::Text("B".into())),
            CollectionItem::new("button").with("#cell", Scalar::Text("C".into())),
        ],
    );

    let bound = bind(&panel, &data, &lib);
    assert_eq!(bound.children.len(), 3, "one instance per collection index");
    let cells: Vec<&Value> = bound.children.iter().map(|c| prop(c, "text")).collect();
    assert_eq!(cells, [&json!("A"), &json!("B"), &json!("C")]);
}

#[test]
fn factory_selects_the_control_for_each_item_role() {
    let label = ctrl(
        "label_ctrl",
        Some("label"),
        json!({ "text": "#custom_text",
        "bindings": [ { "binding_type": "collection", "binding_collection_name": "custom_form", "binding_name": "#custom_text" } ] }),
    );
    let toggle = ctrl("toggle_ctrl", Some("toggle"), json!({}));
    let lib = StubLibrary(
        [
            ("ns.label_ctrl".to_owned(), label),
            ("ns.toggle_ctrl".to_owned(), toggle),
        ]
        .into_iter()
        .collect(),
    );
    let panel = factory_panel(
        "generated",
        "custom_form",
        &[
            ("label", ControlRef::new("ns", "label_ctrl")),
            ("toggle", ControlRef::new("ns", "toggle_ctrl")),
        ],
    );

    let mut data = DataSource::new();
    data.set_collection(
        "custom_form",
        vec![
            CollectionItem::new("label").with("#custom_text", Scalar::Text("Heading".into())),
            CollectionItem::new("toggle"),
        ],
    );

    let bound = bind(&panel, &data, &lib);
    assert_eq!(bound.children.len(), 2);
    assert_eq!(bound.children[0].control_type.as_deref(), Some("label"));
    assert_eq!(prop(&bound.children[0], "text"), &json!("Heading"));
    assert_eq!(bound.children[1].control_type.as_deref(), Some("toggle"));
}

#[test]
fn collection_details_carries_the_index_into_a_nested_binding() {
    // A wrapper with a collection_details binding, whose deeper child reads the
    // collection at the factory-established index.
    let leaf = ctrl(
        "cell",
        Some("label"),
        json!({
            "text": "#cell_text",
            "bindings": [
                { "binding_type": "collection", "binding_collection_name": "rows", "binding_name": "#cell_text" }
            ],
        }),
    );
    let wrapper = ctrl_children(
        "wrapper",
        Some("panel"),
        json!({
            "bindings": [
                { "binding_type": "collection_details", "binding_collection_name": "rows" }
            ],
        }),
        vec![leaf],
    );
    let lib = StubLibrary([("ns.wrapper".to_owned(), wrapper)].into_iter().collect());
    let panel = factory_panel(
        "rows_panel",
        "rows",
        &[("button", ControlRef::new("ns", "wrapper"))],
    );

    let mut data = DataSource::new();
    data.set_collection(
        "rows",
        vec![
            CollectionItem::new("button").with("#cell_text", Scalar::Text("first".into())),
            CollectionItem::new("button").with("#cell_text", Scalar::Text("second".into())),
        ],
    );

    let bound = bind(&panel, &data, &lib);
    let nested: Vec<&Value> = bound
        .children
        .iter()
        .map(|w| prop(&w.children[0], "text"))
        .collect();
    assert_eq!(nested, [&json!("first"), &json!("second")]);
}

#[test]
fn view_binding_over_a_sibling_drives_visibility() {
    // The vanilla dynamic_button rule: the image shows only for a real, non-loading
    // texture, while its gate panel stays visible for any non-empty texture (so the
    // loading spinner can show).
    let cases = [
        (Scalar::Text("textures/x".into()), true, true),
        (Scalar::Text(String::new()), false, false),
        (Scalar::Text("loading".into()), false, true),
    ];
    for (texture, image_visible, gate_visible) in cases {
        let image = ctrl(
            "image",
            Some("image"),
            json!({
                "bindings": [
                    { "binding_name": "#tex", "binding_name_override": "#texture" },
                    { "binding_type": "view",
                      "source_property_name": "(not ((#texture = '') or (#texture = 'loading')))",
                      "target_property_name": "#visible" }
                ],
            }),
        );
        let gate = ctrl(
            "gate",
            Some("panel"),
            json!({
                "bindings": [
                    { "binding_type": "view", "source_control_name": "image", "resolve_sibling_scope": true,
                      "source_property_name": "(not (#texture = ''))", "target_property_name": "#visible" }
                ],
            }),
        );
        let parent = ctrl_children("row", Some("stack_panel"), json!({}), vec![image, gate]);

        let mut data = DataSource::new();
        data.set_global("#tex", texture);

        let bound = bind(&parent, &data, &EmptyLibrary);
        let img = bound.child("image").unwrap();
        let gt = bound.child("gate").unwrap();
        assert_eq!(img.properties.get("visible"), Some(&json!(image_visible)));
        assert_eq!(gt.properties.get("visible"), Some(&json!(gate_visible)));
    }
}

#[test]
fn empty_texture_binding_emits_no_texture_property() {
    let image = ctrl(
        "image",
        Some("image"),
        json!({
            "bindings": [ { "binding_name": "#tex", "binding_name_override": "#texture" } ],
        }),
    );
    let mut data = DataSource::new();
    data.set_global("#tex", Scalar::Text(String::new()));

    let bound = bind(&image, &data, &EmptyLibrary);
    assert!(
        !bound.properties.contains_key("texture"),
        "empty texture is dropped"
    );

    let mut present = DataSource::new();
    present.set_global("#tex", Scalar::Text("textures/items/apple".into()));
    let shown = bind(&image, &present, &EmptyLibrary);
    assert_eq!(prop(&shown, "texture"), &json!("textures/items/apple"));
}

#[test]
fn strict_screens_hide_unbound_visibility_flags_but_keep_text() {
    let control = ctrl(
        "panel",
        Some("panel"),
        json!({
            "text": "#playername",
            "bindings": [
                { "binding_name": "#store_button_visible", "binding_name_override": "#visible" },
                { "binding_name": "#playername" }
            ],
        }),
    );
    let mut data = DataSource::new();
    let lenient = bind(&control, &data, &EmptyLibrary);
    assert!(!lenient.properties.contains_key("visible"));
    data.set_strict(true);
    let strict = bind(&control, &data, &EmptyLibrary);
    assert_eq!(prop(&strict, "visible"), &json!(false));
    assert_eq!(
        prop(&strict, "text"),
        &json!(""),
        "unbound text is not \"false\""
    );
}

#[test]
fn radio_selection_and_distant_view_sources_drive_tab_content() {
    let tab = |name: &str, index: u64| {
        ctrl(
            name,
            Some("toggle"),
            json!({
                "radio_toggle_group": true, "toggle_name": "navigation_tab",
                "toggle_group_forced_index": index
            }),
        )
    };
    let tabs = ctrl_children(
        "tabs",
        Some("stack_panel"),
        json!({}),
        vec![tab("worlds_toggle", 0), tab("servers_toggle", 2)],
    );
    let content = ctrl(
        "servers_content",
        Some("panel"),
        json!({ "bindings": [ { "binding_type": "view", "source_control_name": "servers_toggle",
            "source_property_name": "#toggle_state", "target_property_name": "#visible" } ] }),
    );
    let body = ctrl_children("body", Some("panel"), json!({}), vec![content]);
    let screen = ctrl_children("screen", Some("panel"), json!({}), vec![tabs, body]);
    let mut data = DataSource::new();
    data.select_radio("navigation_tab", 2);
    let bound = bind(&screen, &data, &EmptyLibrary);
    let tabs = bound.child("tabs").unwrap();
    assert_eq!(
        prop(tabs.child("servers_toggle").unwrap(), "#toggle_state"),
        &json!(true)
    );
    assert_eq!(
        prop(tabs.child("worlds_toggle").unwrap(), "#toggle_state"),
        &json!(false)
    );
    let content = bound
        .child("body")
        .unwrap()
        .child("servers_content")
        .unwrap();
    assert_eq!(prop(content, "visible"), &json!(true));
}
