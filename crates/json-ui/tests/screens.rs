//! Container screens against the real vanilla templates. The `.local` pack is
//! gitignored, so each test skips (not fails) when it is absent.

use std::path::PathBuf;

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
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../.local/assets/bedrock-samples/v1.26.30.32-preview/full/resource_pack/ui");
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
    data.set_collection("container_items", items(27, true));
    data.set_collection("inventory_items", items(27, false));
    data.set_collection("hotbar_items", items(9, false));
    data
}

#[test]
fn the_hud_screen_is_refused() {
    let Some(catalog) = catalog() else {
        eprintln!("skipping: vanilla ui assets not present");
        return;
    };
    let rendered = render_screen(
        "hud.hud_screen",
        &catalog,
        &Context::desktop(),
        &DataSource::new(),
        [480.0, 270.0],
        &env(),
        &ViewState::default(),
    );
    assert!(
        rendered.is_none(),
        "the gameplay HUD never routes through the engine"
    );
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
