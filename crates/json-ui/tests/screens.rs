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

// Vanilla screen roots carry their pack-declared settings over the parser defaults.
#[test]
fn vanilla_screen_settings_come_from_the_pack() {
    let Some(catalog) = catalog() else { return };
    let context = Context::desktop();
    let hud = json_ui::screen_settings("hud.hud_screen", &catalog, &context).unwrap();
    assert!(!hud.absorbs_input && !hud.is_showing_menu && hud.should_steal_mouse);
    assert!(hud.low_frequency_rendering && hud.render_only_when_topmost);
    let toast = json_ui::screen_settings("toast_screen.toast_screen", &catalog, &context).unwrap();
    assert!(toast.always_accepts_input && toast.screen_draws_last && toast.screen_not_flushable);
    assert!(!toast.render_only_when_topmost && toast.is_modal);
    let furnace = json_ui::screen_settings("furnace.furnace_screen", &catalog, &context).unwrap();
    assert!(furnace.close_on_player_hurt && furnace.absorbs_input);
    let pause = json_ui::screen_settings("pause.pause_screen", &catalog, &context).unwrap();
    assert!(pause.cache_screen && pause.is_showing_menu && !pause.should_steal_mouse);
    let dialog = json_ui::screen_settings("common.render_below_base_screen", &catalog, &context);
    assert!(dialog.unwrap().force_render_below);
    assert!(json_ui::screen_settings("hud.hud_content", &catalog, &context).is_none());
}
