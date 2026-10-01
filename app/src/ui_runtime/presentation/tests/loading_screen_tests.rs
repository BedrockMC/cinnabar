//! Joining shows vanilla's world-loading progress screen for the dimension.

use json_ui::Draw;

use super::engine_hud_tests::{engine_presentation, engine_presentation_with};
use super::*;
use crate::ui_runtime::presentation::LoadingStage;

fn texts(presentation: &UiPresentationRuntime) -> Vec<String> {
    presentation
        .loading_draw_nodes()
        .iter()
        .filter_map(|node| match &node.draw {
            Draw::Text { text, .. } => Some(text.clone()),
            _ => None,
        })
        .collect()
}

fn has_sprite(presentation: &UiPresentationRuntime, wanted: &str) -> bool {
    presentation
        .loading_draw_nodes()
        .iter()
        .any(|node| matches!(&node.draw, Draw::Sprite { texture, .. } if texture == wanted))
}

#[test]
fn loading_screen_names_the_join_stage_over_the_dimensions_backdrop() {
    let Some(mut presentation) = engine_presentation() else {
        return;
    };
    let runtime = UiRuntime::new(1);
    presentation.set_loading_stage(Some(LoadingStage::Connecting));
    presentation
        .build(&runtime, 0, [1280, 720], DpiScale::new(1.0).unwrap())
        .unwrap();
    let shown = texts(&presentation);
    assert!(
        shown.iter().any(|text| text == "Locating server"),
        "{shown:?}"
    );
    assert!(
        shown
            .iter()
            .any(|text| text == "Connecting to external server")
    );
    assert!(has_sprite(&presentation, "textures/blocks/dirt"));
    presentation.set_loading_stage(Some(LoadingStage::BuildingTerrain));
    presentation.hud_frame_mut().dimension = 1;
    presentation
        .build(&runtime, 0, [1280, 720], DpiScale::new(1.0).unwrap())
        .unwrap();
    let shown = texts(&presentation);
    for wanted in ["Generating World", "Building terrain"] {
        assert!(shown.iter().any(|text| text == wanted), "{shown:?}");
    }
    assert!(has_sprite(&presentation, "textures/blocks/netherrack"));
}

/// Local-only: writes `loading_screen.png` when `CINNABAR_FORM_SNAPSHOT_DIR` is set.
#[test]
fn loading_screen_snapshot() {
    let Some(mut presentation) =
        engine_presentation_with(super::super::forms::pack_harness::font())
    else {
        return;
    };
    presentation.set_loading_stage(Some(LoadingStage::BuildingTerrain));
    let input = presentation
        .build(
            &UiRuntime::new(1),
            0,
            [1280, 720],
            DpiScale::new(1.0).unwrap(),
        )
        .unwrap();
    super::super::forms::snapshot::write(&input, "loading_screen");
}
