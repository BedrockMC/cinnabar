//! The gameplay HUD against the real vanilla templates. The `.local` pack is
//! gitignored, so each test skips (not fails) when it is absent.

use std::path::PathBuf;

use json_ui::{
    BossBar, Catalog, Context, Draw, DrawNode, HUD_SCREEN, HudModel, HudSlot, HudTitle, LayoutEnv,
    Sidebar, TextMeasure, TextureMeta, TextureSource, Timed, ViewState, hud_context,
    hud_data_source, parse_texture_meta, render_screen,
};

fn pack() -> Option<PathBuf> {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../.local/assets/bedrock-samples/v1.26.30.32-preview/full/resource_pack");
    dir.is_dir().then_some(dir)
}

/// Six virtual px per character, nine per line.
struct FixedText;
impl TextMeasure for FixedText {
    fn extent(&self, text: &str) -> [f64; 2] {
        if text.is_empty() {
            return [0.0, 0.0];
        }
        let lines = text.split('\n');
        let width = lines
            .clone()
            .map(|line| line.chars().count())
            .max()
            .unwrap_or(0);
        [width as f64 * 6.0, lines.count() as f64 * 9.0]
    }
}

/// Texture sizes read from the pack's png headers and json sidecars, cached.
struct PackTextures(
    PathBuf,
    std::cell::RefCell<std::collections::HashMap<String, Option<TextureMeta>>>,
);
impl TextureSource for PackTextures {
    fn texture(&self, path: &str) -> Option<TextureMeta> {
        if let Some(meta) = self.1.borrow().get(path) {
            return *meta;
        }
        let meta = self.read(path);
        self.1.borrow_mut().insert(path.to_owned(), meta);
        meta
    }
}

impl PackTextures {
    fn new(dir: PathBuf) -> Self {
        Self(dir, Default::default())
    }

    fn read(&self, path: &str) -> Option<TextureMeta> {
        let stem = path.trim_end_matches(".png");
        if let Ok(text) = std::fs::read_to_string(self.0.join(format!("{stem}.json")))
            && let Ok(value) = serde_json::from_str::<serde_json::Value>(&text)
            && let Some(meta) = parse_texture_meta(&value)
        {
            return Some(meta);
        }
        let bytes = std::fs::read(self.0.join(format!("{stem}.png"))).ok()?;
        let dimension = |at: usize| -> Option<f64> {
            Some(f64::from(u32::from_be_bytes(
                bytes.get(at..at + 4)?.try_into().ok()?,
            )))
        };
        Some(TextureMeta {
            base_size: [dimension(16)?, dimension(20)?],
            nineslice: None,
        })
    }
}

fn model() -> HudModel {
    HudModel {
        survival_ui: true,
        armor_visible: true,
        hotbar_visible: true,
        xp_bar: true,
        exp_progress: 0.5,
        level: 7,
        hotbar: (0..9)
            .map(|index| HudSlot {
                icon: (index == 0).then_some(0),
                count: if index == 0 { 12 } else { 0 },
                selected: index == 2,
                durability: None,
            })
            .collect(),
        chat_visible: true,
        chat_lifetime: 10.0,
        chat_background_opacity: 0.5,
        chat: vec![Timed {
            text: "hello".into(),
            born: 0.0,
        }],
        title: Some(HudTitle {
            title: "Title".into(),
            subtitle: "Sub".into(),
            fade_in: 0.5,
            stay: 3.5,
            fade_out: 1.0,
            background_alpha: 0.0,
            born: 0.0,
        }),
        actionbar: Some(Timed {
            text: "bar".into(),
            born: 0.0,
        }),
        sidebar: Some(Sidebar {
            title: "Kills".into(),
            rows: vec![("Steve".into(), "3".into()), ("Alex".into(), "1".into())],
            background_opacity: 0.3,
            title_background_opacity: 0.4,
        }),
        boss_bars: vec![BossBar {
            name: "Wither".into(),
            progress: 0.75,
            color: "#aa00aa".into(),
            notches: 0,
        }],
        ..HudModel::default()
    }
}

/// The built-in Java HUD pack's files, as the client layers them.
fn java_pack() -> Vec<(String, Vec<u8>)> {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../assets/java-hud");
    ["ui/hud_screen.json", "ui/scoreboards.json"]
        .into_iter()
        .map(|path| {
            (
                path.to_owned(),
                std::fs::read(dir.join(path)).expect("pack file"),
            )
        })
        .collect()
}

fn render(model: &HudModel) -> Option<Vec<DrawNode>> {
    render_with(model, false)
}

fn render_with(model: &HudModel, java: bool) -> Option<Vec<DrawNode>> {
    let dir = pack()?;
    let mut catalog = Catalog::load_dir(&dir.join("ui")).expect("vanilla ui loads");
    if java {
        let files = java_pack();
        let before = catalog.diagnostics().len();
        catalog.apply_pack(
            files
                .iter()
                .map(|(path, bytes)| (path.as_str(), bytes.as_slice())),
        );
        let notes = &catalog.diagnostics()[before..];
        assert!(notes.is_empty(), "java pack diagnostics: {notes:?}");
    }
    let textures = PackTextures::new(dir);
    let env = LayoutEnv {
        text: &FixedText,
        textures: &textures,
    };
    let render = render_screen(
        HUD_SCREEN,
        &catalog,
        &hud_context(&Context::desktop()),
        &hud_data_source(model),
        [480.0, 270.0],
        &env,
        &ViewState::default(),
    )
    .expect("hud renders");
    Some(render.nodes)
}

fn named<'a>(nodes: &'a [DrawNode], name: &str) -> Vec<&'a DrawNode> {
    nodes.iter().filter(|node| node.name == name).collect()
}

fn dump(nodes: &[DrawNode]) {
    if std::env::var_os("HUD_DUMP").is_some() {
        for node in nodes {
            eprintln!(
                "{:40} {:7.1} {:7.1} {:6.1} {:6.1} a={:.2} f={} {:?}",
                node.name,
                node.dest.x,
                node.dest.y,
                node.dest.w,
                node.dest.h,
                node.alpha,
                node.fades.len(),
                match &node.draw {
                    Draw::Text { text, .. } => format!("text {text:?}"),
                    Draw::Sprite { texture, .. } => texture.clone(),
                    Draw::Custom { renderer, .. } => format!("custom {renderer}"),
                    Draw::Solid { .. } => "solid".into(),
                }
            );
        }
    }
}

#[test]
fn vanilla_hud_draws_its_bound_surfaces() {
    let Some(nodes) = render(&model()) else {
        return;
    };
    dump(&nodes);
    let customs: Vec<&str> = nodes
        .iter()
        .filter_map(|node| match &node.draw {
            Draw::Custom { renderer, .. } => Some(renderer.as_str()),
            _ => None,
        })
        .collect();
    for renderer in [
        "heart_renderer",
        "hunger_renderer",
        "armor_renderer",
        "hotbar_renderer",
    ] {
        assert!(
            customs.contains(&renderer),
            "{renderer} missing: {customs:?}"
        );
    }
    assert_eq!(
        customs
            .iter()
            .filter(|name| **name == "hotbar_renderer")
            .count(),
        9
    );
    let texts: Vec<&str> = nodes
        .iter()
        .filter_map(|node| match &node.draw {
            Draw::Text { text, .. } => Some(text.as_str()),
            _ => None,
        })
        .collect();
    for text in [
        "Title", "Sub", "bar", "hello", "Kills", "Steve", "3", "Wither", "7", "12",
    ] {
        assert!(texts.contains(&text), "{text} missing: {texts:?}");
    }
    // The selected slot frame sits over the third cell.
    let selected = named(&nodes, "hotbar_slot_selected_image");
    assert_eq!(selected.len(), 1);
    // Title and chat carry their fades.
    assert!(
        named(&nodes, "title")
            .iter()
            .all(|node| !node.fades.is_empty())
    );
}

fn text_node<'a>(nodes: &'a [DrawNode], text: &str) -> &'a DrawNode {
    nodes
        .iter()
        .find(|node| matches!(&node.draw, Draw::Text { text: drawn, .. } if drawn == text))
        .unwrap_or_else(|| panic!("no text {text:?}"))
}

fn custom<'a>(nodes: &'a [DrawNode], renderer: &str) -> Vec<&'a DrawNode> {
    nodes
        .iter()
        .filter(
            |node| matches!(&node.draw, Draw::Custom { renderer: drawn, .. } if drawn == renderer),
        )
        .collect()
}

fn at(node: &DrawNode) -> [f64; 2] {
    [node.dest.x, node.dest.y]
}

// Java Gui geometry on a 480x270 GUI-px screen (centre 240, bottom 270).
#[test]
fn java_pack_places_the_hud_where_java_does() {
    let Some(nodes) = render_with(&model(), true) else {
        return;
    };
    dump(&nodes);
    // Status rows: hearts from (c-91, H-39); hunger's right end at c+90.
    assert_eq!(at(custom(&nodes, "heart_renderer")[0]), [149.0, 231.0]);
    assert_eq!(at(custom(&nodes, "armor_renderer")[0]), [149.0, 231.0]);
    assert_eq!(at(custom(&nodes, "hunger_renderer")[0]), [330.0, 231.0]);
    // Hotbar cells from c-90 at H-22, the selection frame 1 px out.
    let slots = custom(&nodes, "hotbar_renderer");
    assert_eq!(slots.len(), 9);
    assert_eq!(at(slots[0]), [150.0, 248.0]);
    assert_eq!(at(slots[8]), [310.0, 248.0]);
    let selected = named(&nodes, "hotbar_slot_selected_image");
    assert_eq!(selected.len(), 1);
    assert_eq!(at(selected[0]), [188.0, 247.0]);
    let icons = custom(&nodes, "inventory_item_renderer");
    assert_eq!(at(icons[0]), [152.0, 251.0]);
    // XP bar 182x5 at H-29; level text top at H-35, outlined four ways.
    let bar = named(&nodes, "empty_progress_bar")
        .into_iter()
        .find(|node| node.dest.y > 200.0)
        .expect("xp bar");
    assert_eq!([bar.dest.x, bar.dest.y], [149.0, 241.0]);
    let level: Vec<_> = nodes
        .iter()
        .filter(|node| matches!(&node.draw, Draw::Text { text, .. } if text == "7"))
        .collect();
    assert_eq!(level.len(), 5);
    assert_eq!(at(level[4]), [237.0, 235.0]);
    // Count text right-aligned in the cell: right edge at cell + 19, top at +12.
    let count = text_node(&nodes, "12");
    assert_eq!(count.dest.x + count.dest.w, 169.0);
    assert_eq!(count.dest.y, 260.0);
    // Titles: 4x from H/2-40, subtitle 2x from H/2+10, action bar top at H-72.
    let title = text_node(&nodes, "Title");
    assert_eq!([title.dest.y, title.dest.h], [95.0, 36.0]);
    assert_eq!(title.dest.x + title.dest.w / 2.0, 240.0);
    assert_eq!(text_node(&nodes, "Sub").dest.y, 145.0);
    assert_eq!(text_node(&nodes, "bar").dest.y, 198.0);
    // Chat: one line whose bottom sits 40 px above the screen bottom, text x 4.
    let chat = text_node(&nodes, "hello");
    assert_eq!([chat.dest.x, chat.dest.y + 9.0], [4.0, 230.0]);
    // Boss bar: name from y 3, bar at y 12, both centred.
    assert_eq!(text_node(&nodes, "Wither").dest.y, 3.0);
    let boss = named(&nodes, "empty_progress_bar")
        .into_iter()
        .find(|node| node.dest.y < 50.0)
        .expect("boss bar");
    assert_eq!(boss.dest.y, 12.0);
    // Sidebar: two rows, box bottom at H/2 + 18/3, right edge at W-1.
    let kills = text_node(&nodes, "Kills");
    let steve = text_node(&nodes, "Steve");
    let score = text_node(&nodes, "3");
    assert!(
        (steve.dest.y + 18.0 - 141.0).abs() < 1e-3,
        "{}",
        steve.dest.y
    );
    assert!((kills.dest.y - (steve.dest.y - 9.0)).abs() < 1e-6);
    assert_eq!(score.dest.x + score.dest.w, 477.0);
    // Titles and the action bar draw without Bedrock's text background.
    assert!(!nodes.iter().any(|node| matches!(
        &node.draw,
        Draw::Sprite { texture, .. } if texture == "textures/ui/hud_tip_text_background"
    )));
    assert_eq!(
        text_node(&nodes, "Alex").dest.x,
        text_node(&nodes, "Steve").dest.x
    );
}

// Phase costs of one HUD frame, printed for profiling (HUD_TIMING=1).
#[test]
fn hud_phase_timing() {
    if std::env::var_os("HUD_TIMING").is_none() {
        return;
    }
    let Some(dir) = pack() else {
        return;
    };
    let mut catalog = Catalog::load_dir(&dir.join("ui")).expect("vanilla ui loads");
    let files = java_pack();
    catalog.apply_pack(
        files
            .iter()
            .map(|(path, bytes)| (path.as_str(), bytes.as_slice())),
    );
    let textures = PackTextures::new(dir);
    let env = LayoutEnv {
        text: &FixedText,
        textures: &textures,
    };
    let context = hud_context(&Context::desktop());
    let model = model();
    let data = hud_data_source(&model);
    let cache = json_ui::ResolveCache::default();
    for round in 0..3 {
        let started = std::time::Instant::now();
        let root = json_ui::resolve(&catalog, HUD_SCREEN, &context)
            .control
            .unwrap();
        let resolved = started.elapsed();
        fn count(control: &json_ui::ResolvedControl) -> (usize, usize, usize) {
            let scope = control
                .properties
                .get("factory_scope")
                .map_or(0, |scope| scope.to_string().len());
            control.children.iter().map(count).fold(
                (
                    1,
                    serde_json::to_string(&control.properties).map_or(0, |text| text.len()),
                    scope,
                ),
                |acc, (nodes, bytes, scope)| (acc.0 + nodes, acc.1 + bytes, acc.2 + scope),
            )
        }
        if round == 0 {
            eprintln!(
                "resolved tree: {:?} (nodes, property bytes, scope bytes)",
                count(&root)
            );
        }
        let library = json_ui::CachedLibrary {
            library: json_ui::CatalogLibrary {
                catalog: &catalog,
                context: &context,
            },
            cache: &cache,
        };
        let started = std::time::Instant::now();
        let loops: usize = std::env::var("HUD_LOOP")
            .ok()
            .and_then(|n| n.parse().ok())
            .unwrap_or(1);
        let root = std::sync::Arc::new(root);
        let mut bound = json_ui::bind_shared(&root, &data, &library);
        for _ in 1..loops {
            bound = json_ui::bind_shared(&root, &data, &library);
        }
        let bound_in = started.elapsed() / loops as u32;
        let started = std::time::Instant::now();
        let mut render =
            json_ui::render_bound(bound.clone(), [480.0, 270.0], &env, &ViewState::default());
        for _ in 1..loops {
            render =
                json_ui::render_bound(bound.clone(), [480.0, 270.0], &env, &ViewState::default());
        }
        let laid = started.elapsed() / loops as u32;
        eprintln!(
            "round {round}: resolve {resolved:?} bind {bound_in:?} layout+emit {laid:?} ({} nodes)",
            render.nodes.len()
        );
    }
}
