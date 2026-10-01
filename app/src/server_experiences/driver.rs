use std::path::PathBuf;
use bevy::{prelude::*, window::PrimaryWindow};
use server_experience::{session::State, trust::{Choice, Settings}};
use crate::{app::ClientFrameSet, menu::MenuRuntime, runtime::network::NetworkHandle,
    ui_runtime::{UiRuntime, presentation::UiPresentationRuntime}};

#[derive(Resource)]
struct ExperienceService {
    settings_path: PathBuf,
    settings: Option<Settings>,
}

/// Registers a silent controller; disk and network work wait for a valid marker.
pub(crate) fn configure(app: &mut App) {
    let settings_path = app.world().resource::<MenuRuntime>().experience_settings_path();
    app.insert_resource(ExperienceService { settings_path, settings: None })
        .add_systems(Update, drive.before(ClientFrameSet::SemanticSample)
            .after(ClientFrameSet::RawInput));
}

/// Handles trusted choices before ordinary UI input, so clicks cannot fall through.
fn drive(
    mut service: ResMut<ExperienceService>,
    mut runtime: ResMut<UiRuntime>,
    mut presentation: ResMut<UiPresentationRuntime>,
    menu: Res<MenuRuntime>,
    network: Res<NetworkHandle>,
    windows: Query<&Window, With<PrimaryWindow>>,
    mut keys: ResMut<ButtonInput<KeyCode>>,
    mut mouse: ResMut<ButtonInput<MouseButton>>,
    time: Res<Time<Real>>,
) {
    let now_ms = u64::try_from(time.elapsed().as_millis()).unwrap_or(u64::MAX);
    let generation = runtime.session_id();
    let extension = &mut runtime.experiences;
    if !extension.handled_marker && let Some(marker) = extension.marker.take() {
        extension.handled_marker = true;
        if let Some(audience) = &extension.audience {
            if service.settings.is_none() { service.settings = Some(Settings::load(&service.settings_path)); }
            if let Some(settings) = &service.settings
                && let Err(error) = extension.session.discover(&marker, audience, settings, super::unix_seconds(), now_ms)
            {
                extension.session.disable();
                bevy::log::warn!(%error, "server experience offer rejected");
            }
        }
    }
    extension.session.tick(super::unix_seconds(), now_ms);
    let prompt = matches!(extension.session.state, State::Offered(_)) && menu.is_visible();
    let focused = windows.single().is_ok_and(|window| window.focused);
    let choice = if focused && keys.just_pressed(KeyCode::F9) {
        Some(Choice::Disable)
    } else if focused && prompt {
        if keys.just_pressed(KeyCode::F6) { Some(Choice::Once) }
        else if keys.just_pressed(KeyCode::F7) { Some(Choice::Always) }
        else if keys.just_pressed(KeyCode::F8) { Some(Choice::Never) }
        else if keys.just_pressed(KeyCode::Escape) { Some(Choice::Cancel) }
        else if mouse.just_pressed(MouseButton::Left) {
            windows.single().ok().and_then(|window| window.cursor_position())
                .and_then(|point| presentation.experience_choice(point.to_array()))
        } else { None }
    } else { None };
    if prompt || choice.is_some() {
        keys.clear();
        mouse.clear();
    }
    if let Some(choice) = choice {
        let result = service.settings.as_mut().map(|settings| extension.session.choose(choice, settings, now_ms));
        match result {
            Some(Ok(true)) => {
                if let Some(settings) = &service.settings
                    && let Err(error) = settings.save(&service.settings_path)
                {
                    extension.session.disable();
                    bevy::log::warn!(%error, "server experience trust could not be saved");
                }
            }
            Some(Err(error)) => {
                extension.session.disable();
                bevy::log::warn!(%error, "server experience choice rejected");
            }
            _ => {}
        }
    }
    if let Some(bytes) = extension.session.take_outbound() {
        let sent = protocol::experience_packet(bytes)
            .is_some_and(|packet| network.send_form_packet(generation, packet).is_ok());
        if !sent { extension.session.disable(); }
    }
    let (text, prompt) = chrome(&extension.session, menu.is_visible());
    if let Err(error) = presentation.set_experience_chrome(text.as_deref(), prompt) {
        extension.session.disable();
        bevy::log::warn!(%error, "server experience trusted UI unavailable");
    }
}

/// Builds plain trusted text; pack data can only fill labeled values.
fn chrome(session: &server_experience::session::Session, in_menu: bool) -> (Option<String>, bool) {
    let text = match &session.state {
        State::Inert | State::Disabled => return (None, false),
        State::Offered(offer) if in_menu => {
            let packages = offer.offer.packages.iter().map(|package| format!("{} ({} bytes)\nPublisher: {}", package.id, package.bytes, package.publisher_key)).collect::<Vec<_>>().join("\n");
            format!("Cinnabar server experience\nServer: {}\nKey: {}\n{}\n{}\nPermissions: {:?}\nMedia/download hosts: {}\nThese hosts see your IP address.\nFallback: {}\nServer code is untrusted. F9 disables it immediately.",
                offer.offer.audience, offer.offer.server_key,
                if session.key_changed { "Server key changed: new approval required." } else { "First-use key pinning does not verify the operator's identity." },
                packages, offer.offer.scope.permissions, offer.offer.scope.origins.iter().cloned().collect::<Vec<_>>().join(", "), offer.offer.fallback)
        }
        State::Offered(_) => "Server experience offered. Pause to review. F9: decline".into(),
        State::Awaiting(_) => "Cinnabar: verifying server experience. F9: disable".into(),
        State::Granted(_) => session.notice.clone().unwrap_or_else(|| "Cinnabar: experience approved; runtime unavailable. F9: disable".into()),
    };
    (Some(text), in_menu && matches!(session.state, State::Offered(_)))
}
