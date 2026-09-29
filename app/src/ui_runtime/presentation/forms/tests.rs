//! Form presentation paths: the fallback without the carrier, and the engine's
//! render caching over a minimal in-memory carrier.
use super::super::{UiPresentationRuntime, tests::fixture_font};
use crate::ui_runtime::UiRuntime;
use assets::{RuntimeUiAssets, UiAtlasPage, UiFile, encode_ui_catalog};
use protocol::{FormKind, FormRequestEvent, ModalDialogForm, ServerFormModel};
use std::sync::Arc;

/// A vanilla-shaped server form screen: a content factory selecting a long form
/// whose buttons come from the `form_buttons` collection.
const SERVER_FORM: &str = r##"{
  "namespace": "server_form",
  "third_party_server_screen": { "type": "screen", "$screen_content": "server_form.main_screen_content" },
  "main_screen_content": { "type": "panel", "size": [0, 0], "controls": [
    { "server_form_factory": { "type": "factory", "control_ids": { "long_form": "@server_form.long_form" } } } ] },
  "long_form": { "type": "stack_panel", "size": [200, "100%c"], "collection_name": "form_buttons",
    "factory": { "name": "buttons", "control_ids": { "button": "@server_form.form_button" } } },
  "form_button": { "type": "button", "size": ["100%", 30],
    "button_mappings": [ { "from_button_id": "button.menu_select", "to_button_id": "button.form_button_click", "mapping_type": "pressed" } ],
    "bindings": [ { "binding_type": "collection_details", "binding_collection_name": "form_buttons" } ],
    "controls": [ { "label": { "type": "label", "text": "#form_button_text", "bindings": [
      { "binding_type": "collection", "binding_collection_name": "form_buttons", "binding_name": "#form_button_text" } ] } } ] }
}"##;

pub(crate) fn mini_carrier() -> Arc<RuntimeUiAssets> {
    let files = [
        ("ui/_global_variables.json", "{}"),
        (
            "ui/_ui_defs.json",
            r#"{ "ui_defs": ["ui/server_form.json"] }"#,
        ),
        ("ui/server_form.json", SERVER_FORM),
    ]
    .map(|(path, text)| UiFile {
        path: path.into(),
        bytes: text.as_bytes().into(),
    });
    let page = UiAtlasPage {
        width: 4,
        height: 4,
        rgba8: vec![255; 64].into(),
    };
    let bytes = encode_ui_catalog([1; 32], &[page], &[], &[], &files).unwrap();
    Arc::new(RuntimeUiAssets::decode(&bytes).unwrap())
}

pub(crate) fn mini_engine_presentation() -> UiPresentationRuntime {
    let mut presentation = UiPresentationRuntime::new(fixture_font()).unwrap();
    presentation.enable_json_ui(mini_carrier()).unwrap();
    presentation
}

// A static form resolves once; only view-state changes re-run layout.
#[test]
fn static_form_resolves_once_and_relayouts_only_on_view_changes() {
    let mut presentation = mini_engine_presentation();
    let mut runtime = super::pack_harness::action_form("Menu", &["A", "B", "C"]);
    let passes = |presentation: &UiPresentationRuntime| {
        presentation
            .form_presentation
            .engine
            .as_ref()
            .unwrap()
            .passes
    };
    for _ in 0..3 {
        presentation
            .build(&runtime, 0, [1280, 720], ui::DpiScale::new(1.0).unwrap())
            .unwrap();
    }
    let identity = runtime.server_forms().active().unwrap().identity;
    let frame = presentation
        .form_engine_frame(identity)
        .expect("engine drew it");
    assert_eq!(frame.hits.len(), 3);
    assert_eq!(passes(&presentation), [1, 1]);
    let key = frame.hits[1].key.clone();
    runtime.server_forms_mut().engine_mut().view.hovered = Some(key);
    for _ in 0..2 {
        presentation
            .build(&runtime, 0, [1280, 720], ui::DpiScale::new(1.0).unwrap())
            .unwrap();
    }
    assert_eq!(passes(&presentation), [1, 2]);
}

#[test]
fn modal_without_the_carrier_uses_the_fallback_with_both_buttons() {
    let mut runtime = UiRuntime::new(1);
    let session = runtime.session_id();
    runtime.server_forms_mut().admit(
        FormRequestEvent {
            form_id: 9,
            kind: FormKind::Modal,
            title: None,
            json: Arc::from("{}"),
            model: ServerFormModel::Modal(ModalDialogForm {
                title: Arc::from("Sure?"),
                content: Arc::from("Body"),
                button1: Arc::from("Yes"),
                button2: Arc::from("No"),
            }),
        },
        1,
        session,
        false,
    );
    let mut presentation = UiPresentationRuntime::new(fixture_font()).unwrap();
    presentation.set_server_ui_pack(&super::ServerUiPack {
        ui_layers: vec![vec![("ui/x.json".to_owned(), b"{}".to_vec())]],
        textures: Vec::new(),
    });
    presentation
        .build(&runtime, 0, [1280, 720], ui::DpiScale::new(1.0).unwrap())
        .unwrap();
    let identity = runtime.server_forms().active().unwrap().identity;
    assert!(presentation.form_engine_frame(identity).is_none());
    assert_eq!(presentation.form_button_count(identity), Some(2));
    assert!(presentation.engine_container_frame().is_none());
}

// The fallback dialog keeps every line of a multi-line label, each its own row
// of text inside the button, with format codes hidden.
#[test]
fn fallback_buttons_show_every_label_line() {
    let runtime = super::pack_harness::action_form(
        "Free For All§zfp0;",
        &["Updates In - 2m 24s\n§cKills - 7\nKillstreak - 1", "Duels"],
    );
    let mut presentation = UiPresentationRuntime::new(fixture_font()).unwrap();
    let nodes = super::pack_harness::render(&mut presentation, &runtime, [1280, 720], 1.0);
    let texts = super::pack_harness::drawn_texts(&nodes);
    for line in [
        "Free For Allfp0;",
        "Updates In - 2m 24s",
        "Kills - 7",
        "Killstreak - 1",
    ] {
        assert!(texts.iter().any(|text| text == line), "{line}: {texts:?}");
    }
    let rows: Vec<f32> = nodes
        .iter()
        .filter(|node| {
            matches!(node.visual(), ui::UiVisual::Text { layout, .. }
                if layout.glyphs().first().is_some_and(|glyph| "UKk".contains(glyph.codepoint)))
        })
        .map(|node| node.bounds().min().y())
        .collect();
    assert_eq!(rows.len(), 3);
    assert!(rows.windows(2).all(|pair| pair[1] > pair[0]), "{rows:?}");
}

// Installing and removing a server pack's UI keeps every published frame
// acceptable to the renderer, which pins the static texture identity and plan.
#[test]
fn server_pack_install_and_removal_keep_the_renderer_accepting_frames() {
    use render::{UiRenderScene, UiRenderStats};
    let mut png = Vec::new();
    image::RgbaImage::from_pixel(16, 8, image::Rgba([9, 8, 7, 255]))
        .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
        .unwrap();
    let pack = super::ServerUiPack {
        ui_layers: vec![vec![(
            "ui/server_form.json".to_owned(),
            br#"{ "namespace": "server_form", "form_button": { "modifications": [
                { "array_name": "controls", "operation": "insert_back", "value": [
                    { "art": { "type": "image", "texture": "textures/ui/pack_button", "size": [16, 8] } } ] } ] } }"#
                .to_vec(),
        )]],
        textures: vec![("textures/ui/pack_button.png".to_owned(), png)],
    };
    let mut presentation = mini_engine_presentation();
    let runtime = super::pack_harness::action_form("Menu", &["A"]);
    let (mut scene, stats) = (UiRenderScene::default(), UiRenderStats::default());
    let dpi = ui::DpiScale::new(1.0).unwrap();
    let mut publish = |presentation: &mut UiPresentationRuntime| {
        let input = presentation.build(&runtime, 0, [1280, 720], dpi).unwrap();
        scene.publish(input, &stats).unwrap();
    };
    publish(&mut presentation);
    presentation.set_server_ui_pack(&pack);
    assert!(
        presentation.server_ui_pages().is_empty(),
        "nothing packs until drawn"
    );
    publish(&mut presentation);
    assert_eq!(
        presentation.server_ui_pages().len(),
        1,
        "the drawn texture packed"
    );
    publish(&mut presentation);
    presentation.set_server_ui_pack(&super::ServerUiPack::default());
    assert!(presentation.server_ui_pages().is_empty());
    publish(&mut presentation);
}
