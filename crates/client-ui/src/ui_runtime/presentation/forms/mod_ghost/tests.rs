use super::*;
use crate::ui_runtime::presentation::forms::{snapshot, tests::mini_engine_presentation};
use render_model::UiRenderInput;

fn presentation() -> UiPresentationRuntime {
    let mut presentation = mini_engine_presentation();
    presentation.set_server_ui_pack(&super::super::ServerUiPack {
        ui_layers: vec![vec![
            (
                "ui/_ui_defs.json".into(),
                br#"{"ui_defs":["ui/hud_screen.json"]}"#.to_vec(),
            ),
            (
                "ui/hud_screen.json".into(),
                br#"{"namespace":"hud","hud_screen":{"type":"screen","absorbs_input":false,"is_showing_menu":false,"size":["100%","100%"]}}"#
                    .to_vec(),
            ),
        ]],
        ..Default::default()
    });
    presentation
}

fn frame(presentation: &mut UiPresentationRuntime, dpi: f32) -> UiRenderInput {
    presentation
        .build(
            &player_state::PlayerState::new(1),
            &UiRuntime::new(1),
            0,
            [800, 600],
            DpiScale::new(dpi).unwrap(),
        )
        .unwrap()
}

#[test]
fn rectangle_renders_four_crisp_edges_and_clears_without_affecting_underlying_hud() {
    for dpi in [1.0, 1.5, 2.0] {
        let mut presentation = presentation();
        let before = snapshot::rasterize(&frame(&mut presentation, dpi));
        presentation
            .set_mod_position_rectangle(Some([250, 120, 390, 480]))
            .unwrap();
        let first = frame(&mut presentation, dpi);
        let after = snapshot::rasterize(&first);
        let marker_vertices: Vec<_> = first
            .vertices
            .iter()
            .filter(|vertex| vertex.color == [242, 122, 99, 255])
            .map(|vertex| vertex.position)
            .collect();
        assert!(
            !marker_vertices.is_empty(),
            "marker must paint: passes {}, vertices {:?}",
            presentation
                .form_presentation
                .mod_ghost
                .as_ref()
                .unwrap()
                .screen
                .passes,
            first.vertices
        );
        for (x, y) in [(250, 120), (320, 120), (389, 320), (320, 479), (250, 320)] {
            assert_eq!(
                after.get_pixel(x, y).0,
                [242, 122, 99, 255],
                "edge at dpi {dpi}, ({x},{y}), marker vertices {marker_vertices:?}"
            );
        }
        for (x, y) in [(320, 121), (388, 320), (320, 478), (251, 320)] {
            assert_eq!(
                after.get_pixel(x, y),
                before.get_pixel(x, y),
                "inside at dpi {dpi}"
            );
        }
        assert_eq!(frame(&mut presentation, dpi), first);
        assert_eq!(
            presentation
                .form_presentation
                .mod_ghost
                .as_ref()
                .unwrap()
                .screen
                .passes,
            1
        );
        presentation.set_mod_position_rectangle(None).unwrap();
        assert_eq!(snapshot::rasterize(&frame(&mut presentation, dpi)), before);
    }
}

#[test]
fn marker_is_hidden_when_an_inventory_or_chat_screen_owns_input() {
    for inventory in [false, true] {
        let mut presentation = presentation();
        let player = player_state::PlayerState::new(1);
        let mut runtime = UiRuntime::new(1);
        runtime.inventory_open = inventory;
        runtime.chat_focused = !inventory;
        let build = |p: &mut UiPresentationRuntime| {
            p.build(
                &player,
                &runtime,
                0,
                [800, 600],
                DpiScale::new(1.0).unwrap(),
            )
            .unwrap()
        };
        let before = snapshot::rasterize(&build(&mut presentation));
        presentation
            .set_mod_position_rectangle(Some([250, 120, 390, 480]))
            .unwrap();
        assert_eq!(snapshot::rasterize(&build(&mut presentation)), before);
    }
}
