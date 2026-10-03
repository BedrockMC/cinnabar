//! Real-carrier storage and support input checks; PNGs use the offline gallery path.

use crate::menu::{
    MenuAction, MenuDialog,
    settings_support::{SupportAction, SupportDialog, SupportLink},
};

#[test]
fn settings_help_uses_rating_prompt_and_licenses_scroll() {
    let player_runtime = player_state::PlayerState::new(1);

    let Some(mut presentation) = super::pack_harness::engine_presentation() else {
        return;
    };
    let mut view = settings();
    view.dialog = Some(MenuDialog::SettingsSupport(SupportDialog::Help));
    let actions = draw(&player_runtime, &mut presentation, &view);
    assert!(
        actions.contains(&MenuAction::SettingsSupport(SupportAction::Open(
            SupportLink::Help
        ))),
        "{actions:?}"
    );
    assert!(actions.contains(&MenuAction::DismissDialog));
    assert_eq!(actions.len(), 2, "modal must own input");
    super::play_flow_snapshots::snapshot(&player_runtime, &view, "settings-help-center");
    view.dialog = Some(MenuDialog::SettingsSupport(SupportDialog::FontLicense));
    assert!(
        draw(&player_runtime, &mut presentation, &view)
            .iter()
            .all(|action| *action == MenuAction::DismissDialog)
    );
    assert!(
        presentation.scroll_menu(ui::UiPoint::new(640.0, 360.0).unwrap(), -8.0, false),
        "font license body must scroll"
    );
    assert!(
        presentation
            .menu_scrolls
            .offsets()
            .values()
            .any(|offset| *offset > 0.0)
    );
    super::play_flow_snapshots::snapshot(&player_runtime, &view, "settings-font-license");
}

use crate::test_support::{draw_menu_actions as draw, settings_view as settings};
