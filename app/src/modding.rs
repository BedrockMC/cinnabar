//! Opt-in component spike. The default client registers no extension runtime.

use std::{
    path::Path,
    time::{Duration, Instant},
};

use bevy::{prelude::*, window::PrimaryWindow};
use mod_host::ModHost;

use crate::{
    app::ClientFrameSet,
    menu::MenuRuntime,
    ui_runtime::{UiRuntime, presentation::UiPresentationRuntime},
};

const COMPONENT_ENV: &str = "CINNABAR_MOD_COMPONENT";
const RELOAD_INTERVAL: Duration = Duration::from_millis(500);

#[derive(Resource)]
struct ModRuntime {
    host: ModHost,
    last_reload: Instant,
}

/// Installs the developer extension only when its component path is explicit.
pub(crate) fn configure_from_environment(app: &mut App) {
    let path = std::env::var_os(COMPONENT_ENV);
    configure(app, path.as_deref().map(Path::new));
}

/// Loads one optional component without changing the vanilla schedule on absence.
fn configure(app: &mut App, path: Option<&Path>) {
    let Some(path) = path else { return };
    match ModHost::load(path) {
        Ok(host) => {
            app.insert_resource(ModRuntime {
                host,
                last_reload: Instant::now(),
            })
            .add_systems(
                Update,
                drive_mod
                    .after(ClientFrameSet::SemanticFinalize)
                    .before(ClientFrameSet::UiPublication),
            );
        }
        Err(error) => eprintln!("Cinnabar extension {} disabled: {error:#}", path.display()),
    }
}

/// Runs the bounded guest and publishes only its validated presentation output.
fn drive_mod(
    mut extension: ResMut<ModRuntime>,
    keys: Res<ButtonInput<KeyCode>>,
    windows: Query<&Window, With<PrimaryWindow>>,
    ui: Res<UiRuntime>,
    menu: Option<Res<MenuRuntime>>,
    mut presentation: ResMut<UiPresentationRuntime>,
) {
    if extension.last_reload.elapsed() >= RELOAD_INTERVAL {
        extension.last_reload = Instant::now();
        if let Err(error) = extension.host.reload_if_changed() {
            eprintln!("Cinnabar extension reload rejected: {error:#}");
        }
    }
    let focused = windows.single().is_ok_and(|window| window.focused);
    let pressed = keybind_allowed(
        focused,
        ui.ui_focused(),
        menu.as_ref().is_some_and(|menu| menu.is_visible()),
    ) && keys.just_pressed(KeyCode::F8);
    if extension.host.is_active()
        && let Err(error) = extension.host.frame(pressed)
    {
        eprintln!("Cinnabar extension callback disabled: {error:#}");
    }
    if let Err(error) = presentation.set_mod_label(extension.host.label()) {
        eprintln!("Cinnabar extension HUD rejected: {error}");
    }
}

/// A mod keybind is unavailable while another UI or an unfocused window owns input.
fn keybind_allowed(window_focused: bool, ui_focused: bool, menu_visible: bool) -> bool {
    window_focused && !ui_focused && !menu_visible
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_mod_path_registers_no_runtime_or_systems() {
        let mut app = App::new();
        configure(&mut app, None);
        assert!(!app.world().contains_resource::<ModRuntime>());
        // Any extension system would fail here: none of its required resources exist.
        app.update();
    }

    #[test]
    fn keybind_respects_existing_input_authority() {
        assert!(keybind_allowed(true, false, false));
        assert!(!keybind_allowed(false, false, false));
        assert!(!keybind_allowed(true, true, false));
        assert!(!keybind_allowed(true, false, true));
    }
}
