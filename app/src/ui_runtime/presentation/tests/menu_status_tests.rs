use ui::{DpiScale, SafeArea};

use super::fixture_font;
use crate::{
    menu::{MenuAction, MenuRuntime, MenuScreen},
    ui_runtime::{UiRuntime, presentation::UiPresentationRuntime},
};

const OBSERVED_KICK: &str = "server disconnected: Cinnabar launcher return check (the server ended the current play session)";

#[test]
fn home_catalog_recovery_stays_inside_the_featured_empty_card() {
    let runtime = UiRuntime::new(1);
    for (width, gui_scale) in [(1280, 2), (1280, 3), (900, 2), (420, 2)] {
        let menu = MenuRuntime::new(true, gui_scale, "Player".to_owned());
        let mut view = menu.view();
        view.catalog_loading = false;
        view.catalog_message = Some("Social: Refresh to try again.".to_owned());
        let mut presentation = UiPresentationRuntime::new(fixture_font()).unwrap();
        presentation.set_gui_scale_preference(Some(gui_scale));
        presentation.set_menu_view(Some(view));
        let input = presentation
            .build(&runtime, 0, [width, 720], DpiScale::new(1.0).unwrap())
            .unwrap();
        // Mirror the unchanged shell's desktop/compact empty-card bounds.
        let (left, top, right, bottom) = if width >= 900 {
            let right_width = (((width - 276) as f32) * 0.36).clamp(300.0, 420.0);
            (
                width as f32 - right_width - 8.0,
                168.0,
                width as f32 - 40.0,
                288.0,
            )
        } else {
            (16.0, 396.0, width as f32 - 16.0, 502.0)
        };
        let body: Vec<_> = input
            .vertices
            .iter()
            .filter(|vertex| {
                vertex.color == [166, 178, 193, 255]
                    && vertex.position[0] >= left + 16.0
                    && vertex.position[1] >= top + 20.0
            })
            .collect();
        assert!(
            !body.is_empty(),
            "recovery text must remain visible at GUI {gui_scale}"
        );
        assert!(
            body.iter()
                .all(|vertex| vertex.position[0] <= right && vertex.position[1] <= bottom),
            "recovery text escaped its card at GUI {gui_scale}"
        );
        let title_bottom = input
            .vertices
            .iter()
            .filter(|vertex| {
                vertex.color == [239, 243, 247, 255]
                    && vertex.position[0] >= left + 16.0
                    && vertex.position[1] >= top
            })
            .map(|vertex| vertex.position[1])
            .fold(f32::NEG_INFINITY, f32::max);
        let body_top = body
            .iter()
            .map(|vertex| vertex.position[1])
            .fold(f32::INFINITY, f32::min);
        assert!(
            body_top >= title_bottom + 6.0,
            "title and recovery need a readable gap at GUI {gui_scale}"
        );
    }
}

fn presented_message(
    physical_size: [u32; 2],
    safe_area: SafeArea,
    reason: &str,
) -> (render::UiRenderInput, usize) {
    let runtime = UiRuntime::new(1);
    let mut menu = MenuRuntime::new(true, 2, "Player".to_owned());
    menu.activate(MenuAction::Navigate(MenuScreen::Play));
    let mut presentation = UiPresentationRuntime::new(fixture_font()).unwrap();
    presentation.set_safe_area(safe_area);
    let mut baseline_view = menu.view();
    baseline_view.message = None;
    presentation.set_menu_view(Some(baseline_view));
    let baseline = presentation
        .build(&runtime, 0, physical_size, DpiScale::new(1.0).unwrap())
        .unwrap();

    assert!(menu.absorb_session_failure(reason));
    presentation.set_menu_view(Some(menu.view()));
    let active = presentation
        .build(&runtime, 0, physical_size, DpiScale::new(1.0).unwrap())
        .unwrap();
    (active, baseline.vertices.len())
}

#[test]
fn observed_launcher_disconnect_message_stays_inside_the_window() {
    let (active, baseline_vertices) = presented_message([1280, 720], SafeArea::ZERO, OBSERVED_KICK);
    let status = &active.vertices[baseline_vertices..];

    assert!(
        status.len() > 8,
        "status must retain its text after two panel quads"
    );
    assert!(
        status.iter().all(|vertex| vertex.position[1] <= 720.0),
        "launcher status escaped the window: {:?}",
        status
            .iter()
            .map(|vertex| vertex.position[1])
            .fold(f32::NEG_INFINITY, f32::max)
    );
    let (panel_top, panel_bottom) = status[..4].iter().fold(
        (f32::INFINITY, f32::NEG_INFINITY),
        |(top, bottom), vertex| (top.min(vertex.position[1]), bottom.max(vertex.position[1])),
    );
    assert!(
        panel_bottom - panel_top > 48.0,
        "multi-row status panel did not expand upward from its former fixed height"
    );
}

#[test]
fn long_launcher_status_keeps_only_complete_rows_inside_a_narrow_safe_viewport() {
    let safe_area = SafeArea::new(18.0, 24.0, 22.0, 36.0).unwrap();
    let reason = "bounded launcher status row ".repeat(40);
    let (active, baseline_vertices) = presented_message([420, 360], safe_area, &reason);
    let status = &active.vertices[baseline_vertices..];
    let safe_bottom = 360.0 - safe_area.bottom();

    assert!(
        status.len() > 8,
        "the constrained viewport still fits status text"
    );
    assert!(
        status
            .iter()
            .all(|vertex| vertex.position[1] <= safe_bottom),
        "status text or panel escaped the safe viewport"
    );
    assert!(
        status.iter().all(|vertex| {
            vertex.position[0] >= safe_area.left()
                && vertex.position[0] <= 420.0 - safe_area.right()
        }),
        "status text or panel escaped the narrow safe width"
    );

    let text = &status[8..];
    let lowest_row_top = text
        .chunks_exact(4)
        .map(|quad| {
            quad.iter()
                .map(|vertex| vertex.position[1])
                .fold(f32::INFINITY, f32::min)
        })
        .fold(f32::NEG_INFINITY, f32::max);
    assert!(
        text.iter()
            .filter(|vertex| vertex.position[1] >= lowest_row_top)
            .all(|vertex| vertex.position[1] <= safe_bottom),
        "the final retained status row must be wholly visible"
    );
}
