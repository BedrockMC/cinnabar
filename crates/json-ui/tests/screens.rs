//! Container screens against the real vanilla templates. The `.local` pack is
//! gitignored, so each test skips (not fails) when it is absent.

mod support;

use json_ui::{
    Catalog, CollectionItem, Context, DataSource, Draw, LayoutEnv, Scalar, TextMeasure,
    TextureMeta, TextureSource, ViewState, render_screen,
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
    let dir = support::vanilla_pack().join("ui");
    dir.is_dir()
        .then(|| Catalog::load_dir(&dir).expect("index files load"))
}

fn items(count: usize, icon: bool) -> Vec<CollectionItem> {
    (0..count)
        .map(|index| {
            let item = CollectionItem::default().with(
                "#inventory_stack_count",
                Scalar::Text(if index == 0 {
                    "5".into()
                } else {
                    String::new()
                }),
            );
            if icon && index == 0 {
                item.with("#item_renderer_data", Scalar::Num(0.0))
            } else {
                item
            }
        })
        .collect()
}

fn chest_data() -> DataSource {
    let mut data = DataSource::new();
    // A screen controller answers the bindings it lacks with false.
    data.set_strict(true);
    data.set_collection("container_items", items(27, true));
    data.set_collection("inventory_items", items(27, false));
    data.set_collection("hotbar_items", items(9, false));
    data
}

#[test]
fn small_chest_exposes_every_slot_by_collection() {
    let Some(catalog) = catalog() else {
        eprintln!("skipping: vanilla ui assets not present");
        return;
    };
    let render = render_screen(
        "chest.small_chest_screen",
        &catalog,
        &Context::desktop(),
        &chest_data(),
        [480.0, 270.0],
        &env(),
        &ViewState::default(),
    )
    .expect("small chest renders");
    let slots = |collection: &str| {
        let mut indices: Vec<usize> = render
            .hits
            .iter()
            .filter(|hit| hit.collection.as_deref() == Some(collection))
            .filter_map(|hit| hit.collection_index)
            .collect();
        indices.sort_unstable();
        indices.dedup();
        indices.len()
    };
    assert_eq!(slots("container_items"), 27);
    assert_eq!(slots("inventory_items"), 27);
    assert_eq!(slots("hotbar_items"), 9);
    assert!(
        render.root_panel.is_some(),
        "the panel bounds outside clicks"
    );
    let renderers: Vec<&str> = render
        .nodes
        .iter()
        .filter_map(|node| match &node.draw {
            Draw::Custom { renderer, data } if data.contains_key("#item_renderer_data") => {
                Some(renderer.as_str())
            }
            _ => None,
        })
        .collect();
    assert_eq!(
        renderers,
        ["inventory_item_renderer"],
        "only the stocked cell draws an item"
    );
    assert_eq!(render.cancel_target.as_deref(), Some("button.menu_exit"));
}

#[test]
fn scene_flags_follow_inheritance_and_context() {
    let mut catalog = Catalog::default();
    catalog.overlay_text(
        "ui/policy.json",
        r#"{
      "namespace": "policy",
      "base": {"type": "screen", "absorbs_input": false,
        "render_game_behind": "$world", "render_only_when_topmost": false},
      "child@base": {},
      "opaque@base": {"absorbs_input": true, "render_game_behind": false},
      "defaults": {"type": "screen"}
    }"#,
    );
    let context = Context::default().with_flag("world", true);
    let settings = |name| {
        json_ui::ScreenSettings::from_root(
            &json_ui::resolve(&catalog, name, &context).control.unwrap(),
        )
    };
    let child = settings("policy.child");
    assert!(!child.absorbs_input);
    assert!(child.render_game_behind);
    assert!(!child.render_only_when_topmost);
    assert!(child.renders(false));
    let opaque = settings("policy.opaque");
    assert!(opaque.absorbs_input);
    assert!(!opaque.render_game_behind);
    assert!(!settings("policy.defaults").renders(false));
    assert!(settings("policy.defaults").renders(true));
    assert_eq!(
        settings("policy.defaults"),
        json_ui::ScreenSettings::default()
    );
}
