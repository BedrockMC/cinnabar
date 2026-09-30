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
      { "binding_type": "collection", "binding_collection_name": "form_buttons", "binding_name": "#form_button_text" } ] } },
      { "image": { "type": "image", "size": [16, 16], "bindings": [
        { "binding_type": "collection", "binding_collection_name": "form_buttons",
          "binding_name": "#form_button_texture", "binding_name_override": "#texture" },
        { "binding_type": "collection", "binding_collection_name": "form_buttons",
          "binding_name": "#form_button_texture_file_system", "binding_name_override": "#texture_file_system" } ] } } ] }
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

// A static form resolves and lays out once; hovering a button only repaints.
#[test]
fn static_form_resolves_once_and_hover_only_repaints() {
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
    assert_eq!(passes(&presentation), [1, 1]);
    let frame = presentation.form_engine_frame(identity).unwrap();
    assert_eq!(
        frame.hits.len(),
        3,
        "gated hover children add no hit regions"
    );
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

fn png(color: [u8; 4]) -> Vec<u8> {
    let mut bytes = Vec::new();
    image::RgbaImage::from_pixel(8, 8, image::Rgba(color))
        .write_to(
            &mut std::io::Cursor::new(&mut bytes),
            image::ImageFormat::Png,
        )
        .unwrap();
    bytes
}

fn sprite_pages(nodes: &[ui::UiNode]) -> Vec<(u16, [u16; 4])> {
    nodes
        .iter()
        .filter_map(|node| match node.visual() {
            ui::UiVisual::Sprite {
                texture_page, uv, ..
            } => Some((*texture_page, *uv)),
            _ => None,
        })
        .collect()
}

// Button images by vanilla path draw from the item icon atlas when it holds the
// texture, else from the local vanilla pack; a URL image shows once downloaded.
#[test]
fn path_and_url_button_images_resolve_like_vanilla() {
    use super::super::IconRef;
    use protocol::FormButtonImage::{Path, Url};
    let vanilla = std::env::temp_dir().join(format!("forms-vanilla-{}", std::process::id()));
    std::fs::create_dir_all(vanilla.join("textures/blocks")).unwrap();
    std::fs::write(
        vanilla.join("textures/blocks/stone.png"),
        png([9, 9, 9, 255]),
    )
    .unwrap();
    let url = format!(
        "{}/remote.png",
        super::remote_images::tests::serve(png([1, 2, 3, 255]))
    );
    let mut presentation = mini_engine_presentation();
    let apple = IconRef {
        page: 0,
        uv: [0, 0, 1, 1],
        glint: false,
    };
    let icons = [("textures/items/apple".to_owned(), apple)].into();
    let engine = presentation.form_presentation.engine.as_mut().unwrap();
    engine.textures.set_fallbacks(icons, vanilla.clone());
    let remote = engine.textures.remote.clone();
    let runtime = super::pack_harness::image_form(
        "Images",
        &["Item", "Block", "Remote"],
        vec![
            Some(Path("textures/items/apple".into())),
            Some(Path("textures/blocks/stone".into())),
            Some(Url(url.as_str().into())),
        ],
    );
    let server_page =
        presentation.textures.dynamic_start() + super::super::dynamic_textures::SERVER_UI_PAGE;
    let frame = |presentation: &mut UiPresentationRuntime| {
        let nodes = super::pack_harness::render(presentation, &runtime, [1280, 720], 1.0);
        sprite_pages(&nodes)
    };
    let first = frame(&mut presentation);
    assert!(
        first.contains(&(0, [0, 0, 1, 1])),
        "the icon atlas draws the item"
    );
    let on_server = |sprites: &[(u16, [u16; 4])]| {
        sprites
            .iter()
            .filter(|(page, _)| usize::from(*page) == server_page)
            .count()
    };
    assert_eq!(
        on_server(&first),
        1,
        "the block decodes from the vanilla pack"
    );
    super::remote_images::tests::settle(&remote, &url);
    let loaded = frame(&mut presentation);
    assert_eq!(on_server(&loaded), 2, "the downloaded image joins it");
    assert_eq!(presentation.server_ui_pages().len(), 1);
    let _ = std::fs::remove_dir_all(vanilla);
}

/// Six virtual px per character, nine per line.
struct FixedText;
impl json_ui::TextMeasure for FixedText {
    fn extent(&self, text: &str) -> [f64; 2] {
        [text.chars().count() as f64 * 6.0, 9.0]
    }
}

struct NoTextures;
impl json_ui::TextureSource for NoTextures {
    fn texture(&self, _: &str) -> Option<json_ui::TextureMeta> {
        Some(json_ui::TextureMeta {
            base_size: [16.0, 16.0],
            nineslice: None,
        })
    }
}

fn pause_texts() -> Option<Vec<String>> {
    let mut view = crate::menu::MenuRuntime::new(true, 2, "Player".to_owned()).view();
    view.screen = crate::menu::MenuScreen::Pause;
    screen_texts(&view)
}

fn screen_texts(view: &crate::menu::MenuView) -> Option<Vec<String>> {
    let carrier = super::pack_harness::carrier()?;
    let catalog = json_ui::Catalog::from_files(
        carrier
            .ui_files()
            .iter()
            .map(|file| (&*file.path, &*file.bytes)),
    )
    .ok()?;
    let screen = super::menu_screens::screen_data(view, &|_| None)?;
    let env = json_ui::LayoutEnv {
        text: &FixedText,
        textures: &NoTextures,
    };
    let render = json_ui::render_screen(
        screen.reference,
        &catalog,
        &screen.context,
        &screen.data,
        [480.0, 270.0],
        &env,
        &json_ui::ViewState::default(),
    )?;
    Some(
        render
            .nodes
            .iter()
            .filter_map(|node| match &node.draw {
                json_ui::Draw::Text { text, .. } => Some(text.clone()),
                _ => None,
            })
            .collect(),
    )
}

// A retail client's pause screen draws the retail content, not edu_pause's.
#[test]
fn pause_screen_draws_the_retail_buttons() {
    let Some(texts) = pause_texts() else {
        return;
    };
    for wanted in ["menu.returnToGame", "menu.settings", "pauseScreen.quit"] {
        assert!(
            texts.iter().any(|text| text == wanted),
            "{wanted}: {texts:?}"
        );
    }
}

// A pack download shows vanilla's "Downloading packs" title with the percent and bytes.
#[test]
fn connecting_screen_reports_the_pack_download() {
    let mut view = crate::menu::MenuRuntime::new(true, 2, "Player".to_owned()).view();
    view.connecting = true;
    view.feeds.pack_download = Some((5 * 1024 * 1024, 20 * 1024 * 1024));
    let Some(texts) = screen_texts(&view) else {
        return;
    };
    for wanted in ["Downloading packs 25%", "5.0 / 20.0 MB"] {
        assert!(
            texts.iter().any(|text| text == wanted),
            "{wanted}: {texts:?}"
        );
    }
}

// Retail desktop settings show vanilla's section set; debug, edu, touch and
// automation sections stay hidden.
#[test]
fn retail_settings_hide_debug_and_automation_sections() {
    let Some(carrier) = super::pack_harness::carrier() else {
        return;
    };
    let files = carrier.ui_files();
    let catalog =
        json_ui::Catalog::from_files(files.iter().map(|file| (&*file.path, &*file.bytes))).unwrap();
    let (reference, context) = super::menu_screens::settings_prewarm();
    let tree = json_ui::resolve(&catalog, reference, &context)
        .control
        .unwrap();
    let mut names = Vec::new();
    let mut stack = vec![&tree];
    while let Some(control) = stack.pop() {
        names.push(control.name.as_str());
        stack.extend(&control.children);
    }
    for shown in [
        "accessibility_button",
        "keyboard_and_mouse_button",
        "controller_button",
        "general_button",
        "video_button",
        "sound_button",
        "account_button",
        "view_subscriptions_button",
        "global_texture_pack_button",
        "storage_management_button",
        "language_button",
        "creator_button",
    ] {
        assert!(names.contains(&shown), "{shown} missing");
    }
    for hidden in [
        "touch_button",
        "switch_controller_button",
        "party_button",
        "preview_button",
        "debug_button",
        "ui_debug_button",
        "edu_debug_button",
        "edu_cloud_storage_button",
        "automation_button",
    ] {
        assert!(!names.contains(&hidden), "{hidden} shown");
    }
}
