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

/// Texture sizes read from the pack's png headers and json sidecars.
struct PackTextures(PathBuf);
impl TextureSource for PackTextures {
    fn texture(&self, path: &str) -> Option<TextureMeta> {
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
        }],
        ..HudModel::default()
    }
}

fn render(model: &HudModel) -> Option<Vec<DrawNode>> {
    let dir = pack()?;
    let catalog = Catalog::load_dir(&dir.join("ui")).expect("vanilla ui loads");
    let textures = PackTextures(dir);
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

#[test]
fn vanilla_hud_draws_its_bound_surfaces() {
    let Some(nodes) = render(&model()) else {
        return;
    };
    if std::env::var_os("HUD_DUMP").is_some() {
        for node in &nodes {
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
