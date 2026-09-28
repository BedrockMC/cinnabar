//! End-to-end form rendering against the real vanilla `server_form.json` templates.
//! The `.local` pack is gitignored, so each test skips (not fails) when it is absent.
//! Assertions are structural (instance counts, order, presence of image/text nodes,
//! content sizing) — never pixels.

use std::path::PathBuf;

use json_ui::{
    ActionForm, ButtonImage, Catalog, Context, CustomElement, CustomForm, Draw, DrawNode,
    FormButton, FormModel, LaidOut, LayoutEnv, ModalForm, ResolvedControl, TextMeasure,
    TextureMeta, TextureSource, layout, render_form,
};

struct ZeroText;
impl TextMeasure for ZeroText {
    fn extent(&self, _text: &str) -> [f64; 2] {
        [0.0, 0.0]
    }
}

struct NoTextures;
impl TextureSource for NoTextures {
    fn texture(&self, _path: &str) -> Option<TextureMeta> {
        None
    }
}

fn env() -> LayoutEnv<'static> {
    LayoutEnv {
        text: &ZeroText,
        textures: &NoTextures,
    }
}

fn catalog() -> Option<Catalog> {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../.local/assets/bedrock-samples/v1.26.30.32-preview/full/resource_pack/ui");
    dir.is_dir()
        .then(|| Catalog::load_dir(&dir).expect("index files load"))
}

/// Depth-first search for the first descendant (or self) with `name`.
fn find<'a>(control: &'a ResolvedControl, name: &str) -> Option<&'a ResolvedControl> {
    control.find(&|node| node.name == name)
}

fn find_laid<'a>(root: &'a LaidOut<'a>, name: &str) -> Option<&'a LaidOut<'a>> {
    if root.control.name == name {
        return Some(root);
    }
    root.children
        .iter()
        .find_map(|child| find_laid(child, name))
}

fn texts(nodes: &[DrawNode]) -> Vec<String> {
    nodes
        .iter()
        .filter_map(|node| match &node.draw {
            Draw::Text { text, .. } => Some(text.clone()),
            _ => None,
        })
        .collect()
}

/// Textures of every sprite emitted by a control instance named `name`.
fn sprite_textures(nodes: &[DrawNode], name: &str) -> Vec<String> {
    nodes
        .iter()
        .filter(|node| node.name == name)
        .filter_map(|node| match &node.draw {
            Draw::Sprite { texture, .. } => Some(texture.clone()),
            _ => None,
        })
        .collect()
}

const ROOT: [f64; 2] = [512.0, 384.0];

#[test]
fn action_form_renders_a_button_per_entry_with_present_images_only() {
    let Some(catalog) = catalog() else {
        eprintln!("skipping: vanilla ui assets not present");
        return;
    };
    let model = FormModel::Action(ActionForm {
        title: "Shop".into(),
        body: "Choose an item".into(),
        buttons: vec![
            FormButton {
                text: "Apple".into(),
                image: Some(ButtonImage::Path("textures/items/apple".into())),
            },
            FormButton {
                text: "Sword".into(),
                image: Some(ButtonImage::Path("textures/items/sword".into())),
            },
            FormButton {
                text: "Plain".into(),
                image: None,
            },
        ],
    });

    let render = render_form(&model, &catalog, &Context::desktop(), ROOT, &env())
        .expect("long_form resolves");

    // The factory instantiates one control per collection index.
    let panel = find(&render.bound, "long_form_dynamic_buttons_panel").expect("buttons panel");
    assert_eq!(panel.children.len(), 3, "one button instance per entry");

    // Every button label reaches the draw tree.
    let drawn = texts(&render.nodes);
    for label in ["Apple", "Sword", "Plain"] {
        assert!(
            drawn.iter().any(|t| t == label),
            "missing button text {label}"
        );
    }

    // Only the two buttons with images emit an `image` sprite, and with their paths.
    let mut images = sprite_textures(&render.nodes, "image");
    images.sort();
    images.dedup();
    assert_eq!(
        images,
        vec![
            "textures/items/apple".to_owned(),
            "textures/items/sword".to_owned()
        ],
        "the imageless button emits no image sprite"
    );
}

#[test]
fn modal_form_renders_two_buttons_via_long_form() {
    let Some(catalog) = catalog() else {
        eprintln!("skipping: vanilla ui assets not present");
        return;
    };
    let model = FormModel::Modal(ModalForm {
        title: "Confirm".into(),
        body: "Delete the world?".into(),
        button1: "Yes".into(),
        button2: "No".into(),
    });

    let render = render_form(&model, &catalog, &Context::desktop(), ROOT, &env())
        .expect("modal renders via long_form");

    let panel = find(&render.bound, "long_form_dynamic_buttons_panel").expect("buttons panel");
    assert_eq!(panel.children.len(), 2, "button1 and button2");

    let drawn = texts(&render.nodes);
    assert!(drawn.iter().any(|t| t == "Yes"));
    assert!(drawn.iter().any(|t| t == "No"));
}

#[test]
fn custom_form_renders_elements_in_order_with_a_submit_button() {
    let Some(catalog) = catalog() else {
        eprintln!("skipping: vanilla ui assets not present");
        return;
    };
    let model = FormModel::Custom(CustomForm {
        title: "Options".into(),
        elements: vec![
            CustomElement::Label {
                text: "Intro".into(),
            },
            CustomElement::Toggle {
                text: "Sound".into(),
                default: true,
            },
            CustomElement::Slider {
                text: "Volume".into(),
                value: 5.0,
            },
            CustomElement::Dropdown {
                text: "Mode".into(),
                index: 0,
            },
            CustomElement::Input {
                text: "Name".into(),
                value: String::new(),
                placeholder: "type".into(),
            },
        ],
        submit_text: "Submit".into(),
        submit_visible: true,
    });

    let render = render_form(&model, &catalog, &Context::desktop(), ROOT, &env())
        .expect("custom_form resolves");

    let generated = find(&render.bound, "generated_form").expect("generated form factory");
    let names: Vec<&str> = generated
        .children
        .iter()
        .map(|child| child.name.as_str())
        .collect();
    assert_eq!(
        names,
        [
            "custom_label",
            "custom_toggle",
            "custom_slider",
            "custom_dropdown",
            "custom_input"
        ],
        "the factory selects each element's control in wire order"
    );

    let submit = find(&render.bound, "submit_button").expect("submit button present");
    assert_eq!(
        submit.properties.get("visible"),
        Some(&serde_json::json!(true)),
        "#submit_button_visible drives the submit button"
    );
}

#[test]
fn custom_form_hides_the_submit_button_when_not_visible() {
    let Some(catalog) = catalog() else {
        eprintln!("skipping: vanilla ui assets not present");
        return;
    };
    let model = FormModel::Custom(CustomForm {
        title: "Options".into(),
        elements: vec![CustomElement::Label {
            text: "Intro".into(),
        }],
        submit_text: "Submit".into(),
        submit_visible: false,
    });
    let render = render_form(&model, &catalog, &Context::desktop(), ROOT, &env()).unwrap();
    let submit = find(&render.bound, "submit_button").expect("submit button present");
    assert_eq!(
        submit.properties.get("visible"),
        Some(&serde_json::json!(false))
    );
}

#[test]
fn scroll_content_height_grows_with_the_button_collection() {
    let Some(catalog) = catalog() else {
        eprintln!("skipping: vanilla ui assets not present");
        return;
    };
    let height = |count: usize| {
        let buttons = (0..count)
            .map(|i| FormButton {
                text: format!("Button {i}"),
                image: None,
            })
            .collect();
        let model = FormModel::Action(ActionForm {
            title: "Menu".into(),
            body: String::new(),
            buttons,
        });
        let render = render_form(&model, &catalog, &Context::desktop(), ROOT, &env()).unwrap();
        let laid = layout(&render.bound, ROOT, &env());
        find_laid(&laid, "long_form_dynamic_buttons_panel")
            .expect("buttons panel laid out")
            .rect
            .h
    };

    // Each dynamic_button is 32 tall and the panel sizes to its `100%c` content.
    assert_eq!(height(1), 32.0);
    assert_eq!(height(3), 96.0);
}
