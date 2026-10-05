//! Opt-in component spike. The default client registers no extension runtime.

#[cfg(feature = "local-mods")]
pub(crate) mod font;

use bevy::prelude::*;
#[cfg(feature = "local-mods")]
use {
    crate::{app::ClientFrameSet, environment::VisualTimeOverride, menu::MenuRuntime},
    bevy::window::{CursorOptions, PrimaryWindow},
    client_ui::ui_runtime::{UiRuntime, presentation::UiPresentationRuntime},
    mod_host::{ModGrants, ModHost},
    std::{
        path::Path,
        time::{Duration, Instant},
    },
};

const COMPONENT_ENV: &str = "CINNABAR_MOD_COMPONENT";
#[cfg(feature = "local-mods")]
const PLAYERS_ENV: &str = "CINNABAR_MOD_PLAYERS";
#[cfg(feature = "local-mods")]
const CAMERA_ENV: &str = "CINNABAR_MOD_CAMERA";
#[cfg(feature = "local-mods")]
const CONTROLS_ENV: &str = "CINNABAR_MOD_CONTROLS";
#[cfg(feature = "local-mods")]
const INTERACTION_ENV: &str = "CINNABAR_MOD_INTERACTION";
#[cfg(feature = "local-mods")]
const SETTINGS_ENV: &str = "CINNABAR_MOD_SETTINGS";
#[cfg(feature = "local-mods")]
const ENTITIES_ENV: &str = "CINNABAR_MOD_ENTITIES";
/// Comma-separated command names the selected component may request.
#[cfg(feature = "local-mods")]
const COMMANDS_ENV: &str = "CINNABAR_MOD_COMMANDS";
#[cfg(feature = "local-mods")]
const DEMO_KEY: KeyCode = KeyCode::F8;
#[cfg(feature = "local-mods")]
const RELOAD_INTERVAL: Duration = Duration::from_millis(500);

#[cfg(feature = "local-mods")]
mod registration;

/// The local mod's presentation cues committed this frame, for renderer-side effects.
#[cfg(feature = "local-mods")]
#[derive(Resource, Default)]
pub(crate) struct ModCueFeed(pub Vec<mod_host::ModCue>);

#[cfg(feature = "local-mods")]
#[derive(Resource)]
struct ModRuntime {
    host: ModHost,
    last_reload: Instant,
    grants: ModGrants,
    controls: mod_host::ControlFrame,
    reload_on_main: bool,
    registration_identity: Option<[u8; 32]>,
    registration_request: Option<(u64, String)>,
    suspended: bool,
}

/// Installs the developer extension only when its component path is explicit.
pub(crate) fn configure_from_environment(app: &mut App) {
    let path = std::env::var_os(COMPONENT_ENV);
    #[cfg(feature = "local-mods")]
    if let Some(path) = path.as_deref() {
        configure(app, Some(Path::new(path)));
    } else {
        match launcher::install_layout::InstallLayout::discover()
            .map_err(|error| error.to_string())
            .and_then(|layout| registration::Watcher::start(layout.user_config_root))
        {
            Ok(watcher) => {
                app.insert_resource(watcher);
                configure_systems(app, true);
            }
            Err(error) => eprintln!("Local extension watcher unavailable: {error}"),
        }
    }
    #[cfg(not(feature = "local-mods"))]
    if path.is_some() {
        let _ = app;
        eprintln!("{COMPONENT_ENV} ignored: build bedrock-client with --features local-mods");
    }
}

/// Loads one optional component without changing the vanilla schedule on absence.
#[cfg(feature = "local-mods")]
fn configure(app: &mut App, path: Option<&Path>) {
    let grants = ModGrants {
        environment: true,
        players: std::env::var(PLAYERS_ENV).is_ok_and(|value| value == "1"),
        camera: std::env::var(CAMERA_ENV).is_ok_and(|value| value == "1"),
        controls: std::env::var(CONTROLS_ENV).is_ok_and(|value| value == "1"),
        interaction: std::env::var(INTERACTION_ENV).is_ok_and(|value| value == "1"),
        settings: std::env::var(SETTINGS_ENV).is_ok_and(|value| value == "1"),
        entities: std::env::var(ENTITIES_ENV).is_ok_and(|value| value == "1"),
        commands: std::env::var(COMMANDS_ENV)
            .map(|names| {
                names
                    .split(',')
                    .filter(|name| !name.is_empty())
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default(),
    };
    configure_with_grants(app, path, grants);
}

/// Grants are explicit and apply only to the selected personal component.
#[cfg(feature = "local-mods")]
fn configure_with_grants(app: &mut App, path: Option<&Path>, grants: ModGrants) {
    let Some(path) = path else { return };
    let controls = grants.controls;
    match ModHost::load_with_grants(path, grants.clone()) {
        Ok(host) => {
            app.insert_resource(VisualTimeOverride(host.time_override()))
                .insert_resource(ModRuntime {
                    host,
                    last_reload: Instant::now(),
                    grants,
                    controls: mod_host::empty_controls(),
                    reload_on_main: true,
                    registration_identity: None,
                    registration_request: None,
                    suspended: false,
                })
                .init_resource::<interaction::ModInteraction>();
            if controls && let Some(path) = std::env::var_os(font::FONT_ENV) {
                match font::load(Path::new(&path)).and_then(|font| {
                    app.world_mut()
                        .get_resource_mut::<UiPresentationRuntime>()
                        .ok_or_else(|| "presentation is unavailable".to_owned())?
                        .set_mod_panel_font(Some(std::sync::Arc::new(font)))
                        .map_err(|error| error.to_string())
                }) {
                    Ok(()) => {}
                    Err(error) => eprintln!("Optional personal-panel font unavailable: {error}"),
                }
            }
            configure_systems(app, false);
        }
        Err(error) => eprintln!("Cinnabar extension {} disabled: {error:#}", path.display()),
    }
}

#[cfg(feature = "local-mods")]
fn configure_systems(app: &mut App, watching: bool) {
    app.init_resource::<ModCueFeed>();
    if watching {
        app.add_systems(
            Update,
            (
                registration::install_pending,
                ApplyDeferred,
                input::prepare_mod_input,
            )
                .chain()
                .before(ClientFrameSet::RawInput),
        );
    } else {
        app.add_systems(
            Update,
            input::prepare_mod_input.before(ClientFrameSet::RawInput),
        );
    }
    app.add_systems(
        Update,
        drive_mod
            .in_set(crate::camera::ModCameraInputSet)
            .after(ClientFrameSet::SemanticFinalize)
            .before(ClientFrameSet::UiPublication)
            .before(crate::environment::update_atmosphere_frame),
    );
}

/// Runs the bounded guest and publishes only its validated presentation output.
#[allow(
    clippy::too_many_arguments,
    reason = "Player authority is borrowed separately from UI state."
)]
#[cfg(feature = "local-mods")]
fn drive_mod(
    player_runtime: bevy::prelude::Res<crate::player_runtime::PlayerRuntime>,
    extension: Option<ResMut<ModRuntime>>,
    keys: Res<ButtonInput<KeyCode>>,
    windows: Query<(&Window, Option<&CursorOptions>), With<PrimaryWindow>>,
    ui: Res<UiRuntime>,
    menu: Option<Res<MenuRuntime>>,
    mut presentation: ResMut<UiPresentationRuntime>,
    time_override: Option<ResMut<VisualTimeOverride>>,
    mut gameplay: gameplay::GameplayContext,
    interaction: Option<ResMut<interaction::ModInteraction>>,
    watcher: Option<Res<registration::Watcher>>,
    mut outputs: (
        Option<Res<crate::runtime::network::NetworkHandle>>,
        Option<ResMut<crate::camera::CameraSettingsAuthority>>,
        Option<ResMut<ModCueFeed>>,
    ),
) {
    let (Some(mut extension), Some(mut time_override), Some(mut interaction)) =
        (extension, time_override, interaction)
    else {
        return;
    };
    if extension.suspended {
        return;
    }
    if extension.reload_on_main && extension.last_reload.elapsed() >= RELOAD_INTERVAL {
        extension.last_reload = Instant::now();
        match extension.host.reload_if_changed() {
            // A new instance starts without its predecessor's cues.
            Ok(true) => {
                if let Some(cues) = outputs.2.as_mut() {
                    cues.0.clear();
                }
            }
            Ok(false) => {}
            Err(error) => eprintln!("Cinnabar extension reload rejected: {error:#}"),
        }
    }
    let focused = windows.single().is_ok_and(|(window, _)| window.focused);
    let absorbed = crate::screen_policy::absorbs_input(
        &player_runtime,
        Some(&ui),
        menu.as_deref(),
        Some(&presentation),
    );
    let pressed = keybind_allowed(focused, absorbed) && keys.just_pressed(DEMO_KEY);
    let captured = windows.single().is_ok_and(|(window, cursor)| {
        cursor.is_some_and(|cursor| crate::camera::input_is_active(window, cursor))
    });
    let snapshot = gameplay.snapshot(captured && !absorbed, &extension.grants);
    let mobs = gameplay.mobs(snapshot.as_ref(), &extension.grants);
    if let Some(feed) = outputs.2.as_ref() {
        extension.host.deliver_cues(feed.0.clone());
    }
    let mut controls = std::mem::replace(&mut extension.controls, mod_host::empty_controls());
    controls.gameplay = snapshot.is_some();
    if extension.host.is_active()
        && let Err(error) = extension
            .host
            .frame_with_world(pressed, snapshot, mobs, controls)
    {
        if let Some((generation, request_id)) = &extension.registration_request
            && let Some(watcher) = watcher.as_ref()
        {
            watcher.quarantine(*generation, request_id.clone(), format!("{error:#}"));
        }
        eprintln!("Cinnabar extension callback failed: {error:#}");
    }
    if let Some(error) = extension.host.take_settings_error() {
        eprintln!("Cinnabar extension preferences could not be saved: {error}");
    }
    if let Some(watcher) = watcher.as_ref() {
        watcher.remember_settings(Some(&extension.host));
    }
    let output = extension.host.take_interaction();
    interaction.attack_reach = output.attack_reach;
    interaction.attack_pulse = output.attack_pulse && gameplay.pulse_attack();
    if let Some(delta) = extension.host.take_camera_delta() {
        gameplay.apply(delta);
    }
    let (network, camera, cues) = outputs;
    send_commands(
        network.as_deref(),
        ui.session_id(),
        extension.host.take_commands(),
    );
    if let Some(mut cues) = cues {
        cues.0 = extension.host.take_cues();
    }
    if let Some(mut camera) = camera {
        camera.set_rig(extension.host.camera_rig().map(camera_rig));
    }
    time_override.0 = extension.host.time_override();
    if let Err(error) = presentation.set_mod_label(extension.host.label()) {
        eprintln!("Cinnabar extension HUD rejected: {error}");
    }
    if let Err(error) = presentation.set_mod_panel(extension.host.panel()) {
        eprintln!("Cinnabar extension panel rejected: {error}");
        extension.host.set_panel_open(false);
    }
    presentation.set_mod_panel_open(extension.host.panel_open());
}

/// Granted commands travel the session-fenced UI packet lane as vanilla command requests.
#[cfg(feature = "local-mods")]
fn send_commands(
    network: Option<&crate::runtime::network::NetworkHandle>,
    session: u64,
    commands: Vec<String>,
) {
    let Some(network) = network else { return };
    for command in commands {
        if network
            .send_form_packet(session, protocol::command_request_packet(&command))
            .is_err()
        {
            eprintln!("Cinnabar extension command dropped: the session is not accepting packets");
            return;
        }
    }
}

#[cfg(feature = "local-mods")]
fn camera_rig(rig: mod_host::GameplayCameraRig) -> crate::camera::CameraRig {
    crate::camera::CameraRig {
        offset: Vec3::new(rig.offset.x, rig.offset.y, rig.offset.z),
        roll_radians: rig.roll,
        fov_delta_degrees: rig.fov_delta,
    }
}

/// A mod keybind is unavailable while another UI or an unfocused window owns input.
#[cfg(feature = "local-mods")]
fn keybind_allowed(window_focused: bool, input_absorbed: bool) -> bool {
    window_focused && !input_absorbed
}

#[cfg(all(test, feature = "local-mods"))]
mod tests {
    use super::*;

    #[test]
    fn no_mod_path_registers_no_runtime_or_systems() {
        let mut app = App::new();
        configure(&mut app, None);
        assert!(!app.world().contains_resource::<ModRuntime>());
        assert!(!app.world().contains_resource::<VisualTimeOverride>());
        // Any extension system would fail here: none of its required resources exist.
        app.update();
    }

    #[test]
    fn keybind_respects_existing_input_authority() {
        assert!(keybind_allowed(true, false));
        assert!(!keybind_allowed(false, false));
        assert!(!keybind_allowed(true, true));
    }

    #[test]
    fn configured_sample_drives_the_app_adapter_offline() {
        if std::env::var_os(COMPONENT_ENV).is_none() {
            eprintln!(
                "skipping configured_sample_drives_the_app_adapter_offline: fixture unavailable; requires CINNABAR_MOD_COMPONENT and installed UI carrier (make assets)"
            );
            return;
        }
        let Some(presentation) =
            crate::ui_runtime::presentation::forms::pack_harness::engine_presentation()
        else {
            return;
        };
        let mut app = App::new();
        app.insert_resource(presentation)
            .insert_resource(UiRuntime::new(1))
            .insert_resource(crate::player_runtime::PlayerRuntime::new(1))
            .insert_resource(ButtonInput::<KeyCode>::default());
        let window = app
            .world_mut()
            .spawn((Window::default(), PrimaryWindow))
            .id();
        let vanilla = sample_frame(&mut app);
        configure_from_environment(&mut app);
        assert!(app.world().contains_resource::<ModRuntime>());
        app.update();
        let initial = sample_frame(&mut app);
        assert_ne!(vanilla, initial);

        app.world_mut().get_mut::<Window>(window).unwrap().focused = false;
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(DEMO_KEY);
        app.update();
        assert_eq!(initial, sample_frame(&mut app));
        app.world_mut().get_mut::<Window>(window).unwrap().focused = true;
        app.update();
        assert_ne!(initial, sample_frame(&mut app));
        assert!(
            app.world()
                .resource::<ButtonInput<KeyCode>>()
                .just_pressed(DEMO_KEY)
        );
    }

    /// Renders the adapter's retained state without a window or network session.
    fn sample_frame(app: &mut App) -> render_model::UiRenderInput {
        let player_runtime = app
            .world()
            .resource::<crate::player_runtime::PlayerRuntime>()
            .clone();
        app.world_mut()
            .resource_mut::<UiPresentationRuntime>()
            .build(
                &player_runtime,
                &UiRuntime::new(1),
                0,
                [1280, 720],
                ui::DpiScale::new(1.0).unwrap(),
            )
            .unwrap()
    }
}

#[cfg(all(test, feature = "local-mods"))]
mod time_changer_tests;

#[cfg(feature = "local-mods")]
mod gameplay;
#[cfg(feature = "local-mods")]
mod input;
#[cfg(feature = "local-mods")]
pub(crate) mod interaction;
