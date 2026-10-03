use super::*;
use std::collections::BTreeMap;

fn files(templates: &[(&str, &str)]) -> screen::Files {
    screen::Files {
        namespace: "demo".into(),
        templates: templates
            .iter()
            .map(|(path, text)| ((*path).to_owned(), text.as_bytes().to_vec()))
            .collect(),
        textures: Vec::new(),
    }
}

/// The carrier's vanilla catalog, as the engine keeps it below every server pack.
fn vanilla() -> Option<Catalog> {
    let carrier = super::super::pack_harness::carrier()?;
    Catalog::from_files(
        carrier
            .ui_files()
            .iter()
            .map(|file| (&*file.path, &*file.bytes)),
    )
    .ok()
}

#[test]
fn templates_resolve_against_vanilla_and_their_own_namespace_only() {
    let Some(vanilla) = vanilla() else {
        return;
    };
    let terminal = r##"{"namespace": "demo",
        "terminal@common.empty_panel": {"controls": [{"row@demo.row": {}}]},
        "row": {"type": "label", "text": "#name"}}"##;
    let catalog = modal_catalog(&vanilla, &files(&[("ui/terminal.json", terminal)])).unwrap();
    assert!(catalog.lookup("demo", "terminal").is_some());
    let foreign = r#"{"namespace": "demo", "terminal@cinnabar_experience.indicator": {}}"#;
    assert!(modal_catalog(&vanilla, &files(&[("ui/terminal.json", foreign)])).is_err());
    let server = r#"{"namespace": "demo", "terminal@my_server_pack.panel": {}}"#;
    assert!(modal_catalog(&vanilla, &files(&[("ui/terminal.json", server)])).is_err());
    let rootless = r#"{"namespace": "demo", "panel": {}}"#;
    assert!(modal_catalog(&vanilla, &files(&[("ui/terminal.json", rootless)])).is_err());
    let mut vanilla_namespace = files(&[(
        "ui/terminal.json",
        r#"{"namespace": "common", "terminal": {}}"#,
    )]);
    vanilla_namespace.namespace = "common".into();
    assert!(modal_catalog(&vanilla, &vanilla_namespace).is_err());
}

/// A 1×1 PNG: signature, IHDR, IDAT and IEND.
const PIXEL: &[u8] = &[
    0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x48, 0x44, 0x52,
    0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00, 0x00, 0x1f, 0x15, 0xc4,
    0x89, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9c, 0x63, 0xf8, 0xcf, 0xc0, 0xf0,
    0x1f, 0x00, 0x05, 0x00, 0x01, 0xff, 0x89, 0x99, 0x3d, 0x1d, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45,
    0x4e, 0x44, 0xae, 0x42, 0x60, 0x82,
];

#[test]
fn bundle_textures_draw_and_nothing_outside_textures_resolves() {
    use json_ui::TextureSource;
    let Some(presentation) = super::super::pack_harness::engine_presentation() else {
        return;
    };
    let engine = presentation.form_presentation.engine.as_deref().unwrap();
    let files = vec![("textures/demo/panel.png".to_owned(), PIXEL.to_vec())];
    let set = engine.textures.confined(&files, 7);
    let atlas = set.lock();
    let view = super::super::textures::Textures {
        assets: engine.assets(),
        set: &set,
        atlas: &atlas,
        images: None,
    };
    assert_eq!(
        view.texture("textures/demo/panel").unwrap().pixels,
        [1.0, 1.0]
    );
    for path in [
        "https://example.com/panel.png",
        "textures/../ui/panel",
        "ui/demo/panel",
        "",
    ] {
        assert!(view.texture(path).is_none(), "{path}");
        assert!(view.missing(path), "{path}");
    }
}

#[test]
fn bound_values_and_rows_reach_the_engine_bindings() {
    let mut modal = screen::Modal::default();
    modal.set_value("#title".into(), screen::Value::Text("§eME".into()));
    modal.set_value(
        "#color".into(),
        screen::Value::Numbers(vec![1.0, 0.5, 0.0, 1.0]),
    );
    modal.set_collection(
        "items".into(),
        vec![BTreeMap::from([
            ("#count".to_owned(), screen::Value::Integer(64)),
            ("#shown".to_owned(), screen::Value::Bool(false)),
        ])],
    );
    let mut expected = DataSource::new();
    expected.set_global("#title", Scalar::Text("§eME".into()));
    expected.set_global(
        "#color",
        Scalar::Json(serde_json::json!([1.0, 0.5, 0.0, 1.0])),
    );
    expected.set_collection(
        "items",
        vec![
            CollectionItem::default()
                .with("#count", Scalar::Int(64))
                .with("#shown", Scalar::Bool(false)),
        ],
    );
    assert_eq!(data_source(&modal), expected);
}
