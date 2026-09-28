//! The server-form input model and the renderer that turns it into a draw-node
//! tree. A [`FormModel`] describes a decoded server form (action, modal, or custom);
//! [`form_data_source`] maps it onto the `#binding` names the vanilla
//! `ui/server_form.json` templates read, and [`render_form`] resolves the matching
//! template, binds it, lays it out, and emits the primitives.
//!
//! Binding names below are read from the vanilla pack's `server_form.json`; the
//! per-index factory role and the button-image routing are behaviour inferences
//! flagged in the crate plan pending confirmation against the running client.

use crate::bind::{CollectionItem, ControlLibrary, DataSource, bind};
use crate::catalog::Catalog;
use crate::emit::{DrawNode, emit};
use crate::layout::{LayoutEnv, layout};
use crate::predicate::Scalar;
use crate::tree::{ControlRef, ResolvedControl};
use crate::{Context, resolve};

/// A decoded server form ready to render.
#[derive(Clone, Debug, PartialEq)]
pub enum FormModel {
    Action(ActionForm),
    Modal(ModalForm),
    Custom(CustomForm),
}

/// A button menu (`type:"form"`): a title, body text, and ordered buttons.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ActionForm {
    pub title: String,
    pub body: String,
    pub buttons: Vec<FormButton>,
}

/// A yes/no dialog (`type:"modal"`): rendered via `long_form` with exactly the two
/// buttons. The wire response is a boolean, encoded on the protocol side.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ModalForm {
    pub title: String,
    pub body: String,
    pub button1: String,
    pub button2: String,
}

/// An input form (`type:"custom_form"`): ordered elements plus the submit button.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CustomForm {
    pub title: String,
    pub elements: Vec<CustomElement>,
    pub submit_text: String,
    pub submit_visible: bool,
}

/// One action-form button: its label and an optional image.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FormButton {
    pub text: String,
    pub image: Option<ButtonImage>,
}

/// A button image source, routed to the two texture bindings the template reads.
#[derive(Clone, Debug, PartialEq)]
pub enum ButtonImage {
    /// A resource-pack texture path → `#form_button_texture`.
    Path(String),
    /// A downloaded/URL texture → `#form_button_texture_file_system`.
    Url(String),
}

/// A custom-form element, in wire order.
#[derive(Clone, Debug, PartialEq)]
pub enum CustomElement {
    Label {
        text: String,
    },
    Header {
        text: String,
    },
    Divider,
    Toggle {
        text: String,
        default: bool,
    },
    Slider {
        text: String,
        value: f64,
    },
    StepSlider {
        text: String,
        index: u32,
    },
    Dropdown {
        text: String,
        index: u32,
    },
    Input {
        text: String,
        value: String,
        placeholder: String,
    },
}

/// The output of rendering a form: the bound tree (for structural inspection) and
/// the flattened draw-node list.
#[derive(Debug)]
pub struct FormRender {
    pub bound: ResolvedControl,
    pub nodes: Vec<DrawNode>,
}

const LONG_FORM: &str = "server_form.long_form";
const CUSTOM_FORM: &str = "server_form.custom_form";

/// The `namespace.name` of the vanilla template a model renders through.
pub fn form_template(model: &FormModel) -> &'static str {
    match model {
        // Modal forms have no dedicated template: the screen factory offers only
        // long_form and custom_form, so a two-button long_form is the modal path.
        FormModel::Action(_) | FormModel::Modal(_) => LONG_FORM,
        FormModel::Custom(_) => CUSTOM_FORM,
    }
}

/// Map a form model onto the `#binding` names its template reads.
pub fn form_data_source(model: &FormModel) -> DataSource {
    let mut data = DataSource::new();
    match model {
        FormModel::Action(form) => {
            long_form_source(&mut data, &form.title, &form.body, &form.buttons)
        }
        FormModel::Modal(form) => {
            let buttons = vec![
                FormButton {
                    text: form.button1.clone(),
                    image: None,
                },
                FormButton {
                    text: form.button2.clone(),
                    image: None,
                },
            ];
            long_form_source(&mut data, &form.title, &form.body, &buttons);
        }
        FormModel::Custom(form) => custom_form_source(&mut data, form),
    }
    data
}

fn long_form_source(data: &mut DataSource, title: &str, body: &str, buttons: &[FormButton]) {
    data.set_global("#title_text", Scalar::Text(title.to_owned()));
    data.set_global("#form_text", Scalar::Text(body.to_owned()));
    data.set_global("#form_button_contents", Scalar::Num(buttons.len() as f64));
    let items = buttons
        .iter()
        .map(|button| {
            let (path, file_system) = match &button.image {
                Some(ButtonImage::Path(path)) => (path.clone(), String::new()),
                Some(ButtonImage::Url(url)) => (String::new(), url.clone()),
                None => (String::new(), String::new()),
            };
            CollectionItem::new("button")
                .with("#form_button_text", Scalar::Text(button.text.clone()))
                .with("#form_button_texture", Scalar::Text(path))
                .with(
                    "#form_button_texture_file_system",
                    Scalar::Text(file_system),
                )
        })
        .collect();
    data.set_collection("form_buttons", items);
}

fn custom_form_source(data: &mut DataSource, form: &CustomForm) {
    data.set_global("#title_text", Scalar::Text(form.title.clone()));
    data.set_global(
        "#custom_form_length",
        Scalar::Num(form.elements.len() as f64),
    );
    data.set_global("#submit_text", Scalar::Text(form.submit_text.clone()));
    data.set_global("#submit_button_visible", Scalar::Bool(form.submit_visible));
    let items = form.elements.iter().map(custom_item).collect();
    data.set_collection("custom_form", items);
}

fn custom_item(element: &CustomElement) -> CollectionItem {
    match element {
        CustomElement::Label { text } => {
            CollectionItem::new("label").with("#custom_text", Scalar::Text(text.clone()))
        }
        CustomElement::Header { text } => {
            CollectionItem::new("header").with("#custom_text", Scalar::Text(text.clone()))
        }
        CustomElement::Divider => CollectionItem::new("divider"),
        CustomElement::Toggle { text, default } => CollectionItem::new("toggle")
            .with("#custom_text", Scalar::Text(text.clone()))
            .with("#custom_toggle_state", Scalar::Bool(*default))
            .with("#custom_toggle_enabled", Scalar::Bool(true)),
        CustomElement::Slider { text, value } => CollectionItem::new("slider")
            .with("#custom_slider_text", Scalar::Text(text.clone()))
            .with("#custom_slider_value", Scalar::Num(*value))
            .with("#custom_slider_enabled", Scalar::Bool(true)),
        CustomElement::StepSlider { text, index } => CollectionItem::new("step_slider")
            .with("#custom_slider_step_text", Scalar::Text(text.clone()))
            .with("#custom_slider_step_value", Scalar::Num(*index as f64)),
        CustomElement::Dropdown { text, index } => CollectionItem::new("dropdown")
            .with("#custom_text", Scalar::Text(text.clone()))
            .with("#custom_dropdown_index", Scalar::Num(*index as f64)),
        CustomElement::Input {
            text,
            value,
            placeholder,
        } => CollectionItem::new("input")
            .with("#custom_text", Scalar::Text(text.clone()))
            .with("#custom_input_text", Scalar::Text(value.clone()))
            .with(
                "#custom_placeholder_text",
                Scalar::Text(placeholder.clone()),
            )
            .with("#custom_input_enabled", Scalar::Bool(true)),
    }
}

/// A [`ControlLibrary`] over the pack catalog: factory `control_ids` resolve as
/// standalone `namespace.name` templates in `context`.
pub struct CatalogLibrary<'a> {
    pub catalog: &'a Catalog,
    pub context: &'a Context,
}

impl ControlLibrary for CatalogLibrary<'_> {
    fn resolve(&self, reference: &ControlRef) -> Option<ResolvedControl> {
        let name = format!("{}.{}", reference.namespace, reference.name);
        resolve(self.catalog, &name, self.context).control
    }
}

/// Resolve the model's template and bind it against the mapped data source,
/// returning the baked tree. `None` when the template reference is unknown.
pub fn bind_form(
    model: &FormModel,
    catalog: &Catalog,
    context: &Context,
) -> Option<ResolvedControl> {
    let root = resolve(catalog, form_template(model), context).control?;
    let data = form_data_source(model);
    let library = CatalogLibrary { catalog, context };
    Some(bind(&root, &data, &library))
}

/// Render a form to its laid-out draw-node tree within a `root_size` virtual screen.
pub fn render_form(
    model: &FormModel,
    catalog: &Catalog,
    context: &Context,
    root_size: [f64; 2],
    env: &LayoutEnv,
) -> Option<FormRender> {
    let bound = bind_form(model, catalog, context)?;
    let nodes = {
        let laid = layout(&bound, root_size, env);
        emit(&laid, env)
    };
    Some(FormRender { bound, nodes })
}
