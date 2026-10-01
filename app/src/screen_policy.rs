//! Shared screen ownership for gameplay consumers outside the semantic router.
use crate::{
    menu::MenuRuntime,
    ui_runtime::{UiRuntime, presentation::UiPresentationRuntime},
};

/// A visible absorbing screen owns all gameplay input, including directly read wheel state.
pub(crate) fn absorbs_input(
    ui: Option<&UiRuntime>,
    menu: Option<&MenuRuntime>,
    presentation: Option<&UiPresentationRuntime>,
) -> bool {
    match (ui, menu, presentation) {
        (Some(ui), Some(menu), Some(presentation)) => presentation.absorbs_gameplay_input(ui, menu),
        _ => ui.is_some_and(UiRuntime::ui_focused) || menu.is_some_and(MenuRuntime::is_visible),
    }
}

/// Full-screen backgrounds and pack scene flags suppress every gameplay rendering path.
pub(crate) fn renders_game(
    ui: Option<&UiRuntime>,
    menu: Option<&MenuRuntime>,
    presentation: Option<&UiPresentationRuntime>,
) -> bool {
    match (ui, menu, presentation) {
        (Some(ui), Some(menu), Some(presentation)) => presentation.renders_game_behind(ui, menu),
        _ => !menu.is_some_and(MenuRuntime::uses_panorama),
    }
}
