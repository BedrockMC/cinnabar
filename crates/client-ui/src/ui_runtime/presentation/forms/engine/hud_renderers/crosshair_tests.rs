use super::*;
use crate::test_support::{fixture_font, mini_carrier};
use crate::ui_runtime::presentation::forms::{
    engine::{EngineInputs, EngineOutput, FormEngine, ScreenArt},
    server_pack::ServerAtlas,
};
use crate::ui_runtime::presentation::{FONT_DESIGN_PIXEL_TEXELS, TextMetrics};
use json_ui::{Catalog, Context, DataSource, ViewState};
use ui::{DpiScale, SafeArea, TextLayoutCache, UiNode};

const FALLBACK: SheetSprite = SheetSprite {
    page: 1,
    uv: {
        let [width, height] = assets::HudTextureRole::Crosshair.expected_size();
        [0, 0, width as u16, height as u16]
    },
};

fn engine(files: &[(String, Vec<u8>)]) -> FormEngine {
    let mut catalog = Catalog::default();
    let (namespace, name) = json_ui::CROSSHAIR_SCREEN.split_once('.').unwrap();
    let definition = serde_json::json!({
        "namespace": namespace,
        (name): {"type": "screen", "controls": [{
            "cursor": {"type": "custom", "renderer": "cursor_renderer",
                "size": [CROSSHAIR_SIDE, CROSSHAIR_SIDE]}
        }]}
    });
    catalog.overlay_text("ui/crosshair_test.json", &definition.to_string());
    let mut engine = FormEngine::new(mini_carrier(), catalog, 2);
    engine.set_server_atlas(ServerAtlas::new(files, None, 1), 3);
    engine
}

fn draw(engine: &FormEngine, visible: bool) -> (Vec<UiNode>, f32) {
    let font = fixture_font();
    let metrics = TextMetrics::for_viewport([1280, 720], DpiScale::new(1.0).unwrap(), None);
    let px = metrics.scale.get() * FONT_DESIGN_PIXEL_TEXELS as f32;
    let hud = HudPaint {
        crosshair: visible.then_some(FALLBACK),
        ..Default::default()
    };
    let mut layouts = TextLayoutCache::new(32, 1024 * 1024);
    let mut nodes = Vec::new();
    let mut next = 1;
    engine
        .render_screen(
            json_ui::CROSSHAIR_SCREEN,
            &DataSource::default(),
            &Context::default(),
            &ViewState::default(),
            ScreenArt {
                hud: Some(&hud),
                ..Default::default()
            },
            EngineInputs {
                layouts: &mut layouts,
                font: &font,
                metrics,
                solid_page: 0,
                safe_area: SafeArea::ZERO,
                content: [1280.0, 720.0],
                translate: &|_| None,
                language: [0; 3],
            },
            EngineOutput {
                nodes: &mut nodes,
                next: &mut next,
                overlay: &[],
            },
        )
        .unwrap()
        .expect("resolved cursor screen");
    (nodes, px)
}

fn texture(color: [u8; 4]) -> Vec<u8> {
    let mut png = Vec::new();
    image::RgbaImage::from_pixel(32, 32, image::Rgba(color))
        .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
        .unwrap();
    png
}

#[test]
fn crosshair_uses_the_pack_texture_without_a_ui_override() {
    let color = [22, 33, 44, 255];
    let mut engine = engine(&[(format!("{CROSSHAIR_TEXTURE}.png"), texture(color))]);
    let (nodes, px) = draw(&engine, true);
    let node = nodes
        .iter()
        .find(|node| matches!(node.visual(), UiVisual::InvertedSprite { .. }))
        .expect("inverting crosshair");
    let UiVisual::InvertedSprite { texture_page, uv } = node.visual() else {
        unreachable!()
    };
    assert_eq!(
        *texture_page, 3,
        "pack texture replaces the pinned HUD sprite"
    );
    assert_eq!(
        [uv[2] - uv[0], uv[3] - uv[1]],
        [32, 32],
        "full texture, not an icons-sheet crop"
    );
    assert_eq!(node.bounds().width(), CROSSHAIR_SIDE * px);
    assert_eq!(node.bounds().height(), CROSSHAIR_SIDE * px);
    let pages = engine
        .take_server_pages()
        .expect("crosshair atlas uploaded");
    let width = pages[0].dimensions()[0] as usize;
    let offset = (usize::from(uv[1]) * width + usize::from(uv[0])) * 4;
    assert_eq!(&pages[0].pixels()[offset..offset + 4], &color);
}

#[test]
fn crosshair_missing_or_invalid_texture_keeps_the_builtin_hud_sprite() {
    for files in [
        Vec::new(),
        vec![(format!("{CROSSHAIR_TEXTURE}.png"), vec![0])],
    ] {
        let engine = engine(&files);
        let (nodes, _) = draw(&engine, true);
        assert!(nodes.iter().any(|node| matches!(node.visual(),
            UiVisual::InvertedSprite { texture_page, uv } if *texture_page == FALLBACK.page && *uv == FALLBACK.uv
        )));
    }
}

#[test]
fn crosshair_pack_texture_does_not_bypass_visibility() {
    let engine = engine(&[(format!("{CROSSHAIR_TEXTURE}.png"), texture([255; 4]))]);
    let (nodes, _) = draw(&engine, false);
    assert!(
        !nodes
            .iter()
            .any(|node| matches!(node.visual(), UiVisual::InvertedSprite { .. }))
    );
}
