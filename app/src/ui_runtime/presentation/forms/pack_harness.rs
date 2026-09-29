//! Local-only harness: renders server forms through the real UI carrier and, when
//! `CINNABAR_FORM_PACK_DIR` names an unpacked server resource pack, its ui overlay.
//! Skips when the gitignored carrier is absent; the pack is never committed.

use std::{collections::BTreeMap, path::Path, sync::Arc};

use assets::{RuntimeFontCatalog, RuntimeUiAssets};
use protocol::{FormKind, FormRequestEvent, ServerFormModel, TextMenuForm, UiEvent};
use ui::{DpiScale, UiNode, UiVisual};

use super::super::{TextMetrics, UiPresentationRuntime, tests::fixture_font};
use super::ServerUiPack;
use crate::ui_runtime::{SequencedUiEvent, UiRuntime};

const PACK_ENV: &str = "CINNABAR_FORM_PACK_DIR";

fn local(path: &str) -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../.local")
        .join(path)
}

pub(crate) fn carrier() -> Option<Arc<RuntimeUiAssets>> {
    let bytes = std::fs::read(local("assets/compiled/vanilla-v1.mcbeui")).ok()?;
    RuntimeUiAssets::decode(&bytes).ok().map(Arc::new)
}

pub(crate) fn font() -> Arc<RuntimeFontCatalog> {
    let manifest = crate::asset_startup::canonical_source_manifest_sha256(include_str!(
        "../../../../../assets/ui-font-source.json"
    ));
    std::fs::read(local("assets/compiled/ui-monocraft-v1.mcbefont"))
        .ok()
        .and_then(|bytes| RuntimeFontCatalog::decode(&bytes, manifest).ok())
        .map_or_else(fixture_font, Arc::new)
}

/// Every file of an unpacked pack directory as `(pack-relative path, bytes)`.
pub(crate) fn pack_files(root: &Path) -> Vec<(String, Vec<u8>)> {
    let mut files = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if let (Ok(relative), Ok(bytes)) =
                (path.strip_prefix(root), std::fs::read(&path))
            {
                files.push((relative.to_string_lossy().replace('\\', "/"), bytes));
            }
        }
    }
    files.sort();
    files
}

/// The packs `CINNABAR_FORM_PACK_DIR` lists (`:`-separated, lowest first).
pub(crate) fn env_pack() -> Option<ServerUiPack> {
    let dirs = std::env::var(PACK_ENV).ok()?;
    let mut pack = ServerUiPack::default();
    let mut all = Vec::new();
    for dir in dirs.split(':').filter(|dir| !dir.is_empty()) {
        let files = pack_files(Path::new(dir));
        pack.ui_layers.push(
            files
                .iter()
                .filter(|(path, _)| path.starts_with("ui/") && path.ends_with(".json"))
                .cloned()
                .collect(),
        );
        all.extend(files);
    }
    let wanted = ServerUiPack::referenced_texture_dirs(&pack.ui_layers);
    let mut textures = BTreeMap::new();
    for (path, bytes) in all {
        if ServerUiPack::wants_texture(&wanted, &path) {
            textures.insert(path, bytes);
        }
    }
    pack.textures = textures.into_iter().collect();
    Some(pack)
}

pub(crate) fn action_form(title: &str, buttons: &[&str]) -> UiRuntime {
    image_form(title, buttons, Vec::new())
}

pub(crate) fn image_form(
    title: &str,
    buttons: &[&str],
    images: Vec<Option<protocol::FormButtonImage>>,
) -> UiRuntime {
    let mut runtime = UiRuntime::new(1);
    runtime
        .apply(SequencedUiEvent {
            session_id: 1,
            fifo_sequence: 1,
            local_millis: 0,
            server_tick: None,
            event: UiEvent::Form(FormRequestEvent {
                form_id: 3,
                kind: FormKind::Menu,
                title: Some(Arc::from(title)),
                json: Arc::from("{}"),
                model: ServerFormModel::TextMenu(TextMenuForm {
                    title: Arc::from(title),
                    content: Arc::from(""),
                    buttons: buttons.iter().map(|text| Arc::from(*text)).collect(),
                    button_images: images.into(),
                    omitted_images: 0,
                }),
            }),
        })
        .unwrap();
    runtime
}

/// Text drawn by `nodes`, rebuilt from each layout's glyph codepoints.
pub(crate) fn drawn_texts(nodes: &[UiNode]) -> Vec<String> {
    nodes
        .iter()
        .filter_map(|node| match node.visual() {
            UiVisual::Text { layout, .. } => Some(
                layout
                    .glyphs()
                    .iter()
                    .map(|glyph| glyph.codepoint)
                    .collect(),
            ),
            _ => None,
        })
        .collect()
}

pub(crate) fn render(
    presentation: &mut UiPresentationRuntime,
    runtime: &UiRuntime,
    size: [u32; 2],
    dpi: f32,
) -> Vec<UiNode> {
    let dpi = DpiScale::new(dpi).unwrap();
    let metrics = TextMetrics::for_viewport(size, dpi, None);
    let (width, height) = (size[0] as f32 / dpi.get(), size[1] as f32 / dpi.get());
    let mut nodes = Vec::new();
    let mut next = 1;
    presentation
        .append_server_form(runtime, &mut nodes, &mut next, metrics, width, height)
        .unwrap();
    presentation.sync_server_ui_pages();
    nodes
}

pub(crate) fn engine_presentation() -> Option<UiPresentationRuntime> {
    let carrier = carrier()?;
    let mut presentation = UiPresentationRuntime::new(font()).unwrap();
    presentation.enable_json_ui(carrier).unwrap();
    let vanilla = local(crate::install_layout::VANILLA_PACK_DIR);
    let engine = presentation.form_presentation.engine.as_mut().unwrap();
    engine.textures.set_fallbacks(Default::default(), vanilla);
    Some(presentation)
}

/// Every text node with its bounds and its clip parent's bounds, for diagnosis.
pub(crate) fn dump(nodes: &[UiNode]) {
    for node in nodes {
        if let UiVisual::Text { layout, color, .. } = node.visual() {
            let text: String = layout
                .glyphs()
                .iter()
                .map(|glyph| glyph.codepoint)
                .collect();
            let clip = node
                .parent()
                .and_then(|parent| nodes.iter().find(|other| other.id() == parent))
                .map(UiNode::bounds);
            eprintln!(
                "{text:?} lines={} size={:?} color={color:?} at {:?} clip {clip:?}",
                layout.line_count(),
                layout.size_64().map(|v| v as f32 / 64.0),
                node.bounds()
            );
        }
    }
}

#[test]
fn server_pack_form_renders_its_text_through_the_engine() {
    let Some(mut presentation) = engine_presentation() else {
        eprintln!("skipping: UI carrier absent");
        return;
    };
    let pack = env_pack();
    if let Some(pack) = &pack {
        presentation.set_server_ui_pack(pack);
    }
    let buttons = [
        "Common Box\n§73 owned",
        "Rare Box",
        "Epic Box",
        "Legendary",
        "Back",
    ];
    let runtime = action_form("@mineville/boxes:Spirit Bundle", &buttons);
    let nodes = render(&mut presentation, &runtime, [2560, 1600], 2.0);
    dump(&nodes);
    let identity = runtime.server_forms().active().unwrap().identity;
    let texts = drawn_texts(&nodes);
    let sprites = nodes
        .iter()
        .filter(|node| matches!(node.visual(), UiVisual::Sprite { .. }))
        .count();
    eprintln!(
        "engine frame: {}, pack: {}, {} nodes, {sprites} sprites, texts: {texts:?}",
        presentation.form_engine_frame(identity).is_some(),
        pack.is_some(),
        nodes.len(),
    );
    assert!(presentation.form_engine_frame(identity).is_some());
    let (drawn, missing) = presentation
        .form_presentation
        .engine
        .as_ref()
        .unwrap()
        .drawn_sprites();
    eprintln!(
        "server pages: {}, sprite textures drawn: {}, outside textures/ui: {:?}, unresolved: {missing:?}",
        presentation.server_ui_pages().len(),
        drawn.len(),
        drawn
            .iter()
            .filter(|key| !key.starts_with("textures/ui/"))
            .collect::<Vec<_>>()
    );
    assert!(
        missing.is_empty(),
        "every drawn image resolves: {missing:?}"
    );
    // Without a pack the vanilla template shows the labels verbatim.
    if pack.is_none() {
        for label in ["Rare Box", "Epic Box", "Legendary"] {
            assert!(texts.iter().any(|text| text.contains(label)), "{label}");
        }
    }
    assert!(!texts.is_empty());
    assert!(
        texts.iter().all(|text| !text.contains('§')),
        "format codes never draw"
    );
}

/// Each text node's visible rect: its bounds offset by, and cut to, its clip group.
fn visible_text_rects(nodes: &[UiNode]) -> Vec<[f32; 4]> {
    nodes
        .iter()
        .filter(|node| matches!(node.visual(), UiVisual::Text { .. }))
        .filter_map(|node| {
            let clip = nodes
                .iter()
                .find(|other| Some(other.id()) == node.parent())?
                .bounds();
            let (min, max) = (clip.min(), clip.max());
            let bounds = node.bounds();
            let rect = [
                (min.x() + bounds.min().x()).max(min.x()),
                (min.y() + bounds.min().y()).max(min.y()),
                (min.x() + bounds.max().x()).min(max.x()),
                (min.y() + bounds.max().y()).min(max.y()),
            ];
            (rect[2] > rect[0] && rect[3] > rect[1]).then_some(rect)
        })
        .collect()
}

// Multi-line labels stack one line apart and drop lines past the label's height
// instead of spilling over the next button.
#[test]
fn multi_line_button_labels_never_overlap() {
    let Some(mut presentation) = engine_presentation() else {
        eprintln!("skipping: UI carrier absent");
        return;
    };
    let buttons = [
        "Free For All\n§7Playing - 12",
        "Updates In - 2m 24s\nKills - 7\nKillstreak - 1",
        "Duels",
    ];
    let runtime = action_form("Free For All§zfp0;", &buttons);
    let nodes = render(&mut presentation, &runtime, [2560, 1600], 2.0);
    dump(&nodes);
    let rects = visible_text_rects(&nodes);
    for (index, a) in rects.iter().enumerate() {
        for b in &rects[index + 1..] {
            let overlap = a[0] < b[2] && b[0] < a[2] && a[1] < b[3] && b[1] < a[3];
            assert!(!overlap, "{a:?} overlaps {b:?}");
        }
    }
    assert!(
        drawn_texts(&nodes)
            .iter()
            .any(|text| text == "Playing - 12")
    );
}

// Per-frame form cost with the render cache versus re-resolving every frame.
#[test]
fn form_frame_cost_with_and_without_the_render_cache() {
    let Some(mut presentation) = engine_presentation() else {
        eprintln!("skipping: UI carrier absent");
        return;
    };
    if let Some(pack) = env_pack() {
        presentation.set_server_ui_pack(&pack);
    }
    let buttons: Vec<String> = (0..20)
        .map(|index| format!("Button {index}\n§7Line two"))
        .collect();
    let labels: Vec<&str> = buttons.iter().map(String::as_str).collect();
    let runtime = action_form("@mineville/boxes:Spirit Bundle", &labels);
    let frame = |presentation: &mut UiPresentationRuntime, cold: bool| {
        if cold {
            presentation
                .form_presentation
                .engine
                .as_mut()
                .unwrap()
                .cache = None;
        }
        let started = std::time::Instant::now();
        render(presentation, &runtime, [2560, 1600], 2.0);
        started.elapsed()
    };
    frame(&mut presentation, true);
    let average = |presentation: &mut UiPresentationRuntime, cold: bool| {
        let total: std::time::Duration = (0..20).map(|_| frame(presentation, cold)).sum();
        total / 20
    };
    let uncached = average(&mut presentation, true);
    let cached = average(&mut presentation, false);
    eprintln!("form frame: re-resolving {uncached:?}, cached {cached:?}");
    assert!(cached < uncached);
}

// The vanilla template draws path and URL button images once they resolve.
#[test]
fn vanilla_form_button_images_resolve() {
    use protocol::FormButtonImage::{Path as ImagePath, Url};
    let Some(mut presentation) = engine_presentation() else {
        eprintln!("skipping: UI carrier absent");
        return;
    };
    let mut png = Vec::new();
    image::RgbaImage::from_pixel(8, 8, image::Rgba([1, 2, 3, 255]))
        .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
        .unwrap();
    let url = format!("{}/icon.png", super::remote_images::tests::serve(png));
    let runtime = image_form(
        "Shop",
        &["Diamond", "Stone", "Remote"],
        vec![
            Some(ImagePath("textures/items/diamond".into())),
            Some(ImagePath("textures/blocks/stone".into())),
            Some(Url(url.as_str().into())),
        ],
    );
    let engine = presentation.form_presentation.engine.as_ref().unwrap();
    let remote = engine.textures.remote.clone();
    render(&mut presentation, &runtime, [1280, 720], 1.0);
    super::remote_images::tests::settle(&remote, &url);
    render(&mut presentation, &runtime, [1280, 720], 1.0);
    let (drawn, missing) = presentation
        .form_presentation
        .engine
        .as_ref()
        .unwrap()
        .drawn_sprites();
    eprintln!("drawn {drawn:?}, unresolved {missing:?}");
    for image in [
        "textures/items/diamond",
        "textures/blocks/stone",
        url.as_str(),
    ] {
        assert!(drawn.iter().any(|key| key == image), "{image} drawn");
    }
    assert!(missing.is_empty());
}
