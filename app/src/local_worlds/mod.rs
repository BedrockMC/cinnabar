//! World management for local single-player worlds served by the core's dragonfly host.
//!
//! [`WorldsMenu`] is a pure screen model; [`LocalWorlds`] wires it to the core's control
//! channel. The menu module embeds it by calling `attach`, `input`, `menu` and `take_ready`.

mod client;
mod form;
mod launch;
mod model;
mod prompt;

use std::{io, path::PathBuf};

use bevy::{
    prelude::{App, MessageReader, Plugin, ResMut, Resource, Update},
    window::WindowFocused,
};

pub(crate) use form::{difficulty_label, game_mode_label, generator_label};
pub(crate) use launch::spawn_core_for_local_worlds;
pub(crate) use model::{Effect, Event, Input, Screen, WorldsMenu};
pub(crate) use prompt::{PromptButton, PromptKind};

use client::WorldsClient;

/// Menu model plus the control-channel worker that serves it.
#[derive(Default, Resource)]
pub(crate) struct LocalWorlds {
    menu: WorldsMenu,
    client: Option<WorldsClient>,
    playing: bool,
}

impl LocalWorlds {
    /// Connects to the core at `socket_dir` and requests the world list.
    pub(crate) fn attach(&mut self, socket_dir: PathBuf) -> io::Result<()> {
        self.client = Some(WorldsClient::spawn(socket_dir)?);
        self.playing = false;
        self.menu = WorldsMenu::default();
        self.input(Input::Refresh);
        self.dispatch(vec![Effect::LoadPrefs]);
        Ok(())
    }

    /// Drops the worker; the core keeps the open world until [`Self::leave_world`].
    pub(crate) fn detach(&mut self) {
        self.client = None;
        self.playing = false;
    }

    pub(crate) fn menu(&self) -> &WorldsMenu {
        &self.menu
    }

    pub(crate) fn input(&mut self, input: Input) {
        let effects = self.menu.update(input);
        self.dispatch(effects);
    }

    /// Applies finished control-channel requests; call once per frame.
    pub(crate) fn pump(&mut self) {
        let Some(client) = &self.client else { return };
        for event in client.drain() {
            let effects = self.menu.apply(event);
            self.dispatch(effects);
        }
    }

    /// Takes the id of a world that finished opening; the caller then joins the game socket.
    pub(crate) fn take_ready(&mut self) -> Option<String> {
        self.menu.take_ready()
    }

    /// Records whether the player is in the local world; only then does focus loss pause it.
    pub(crate) fn set_playing(&mut self, playing: bool) {
        self.playing = playing;
    }

    /// Leaves the local world: the core saves and stops it.
    pub(crate) fn leave_world(&mut self) {
        self.playing = false;
        self.dispatch(vec![Effect::Close]);
    }

    /// Pauses the world when the window loses focus and resumes it on regain.
    pub(crate) fn focus_changed(&mut self, focused: bool) {
        if self.playing {
            self.dispatch(vec![Effect::SetPaused(!focused)]);
        }
    }

    fn dispatch(&self, effects: Vec<Effect>) {
        for effect in effects {
            if let Effect::OpenUrl(url) = effect {
                open_url(url);
            } else if let Some(client) = &self.client {
                client.send(effect);
            }
        }
    }
}

/// Opens a fixed https URL in the system browser; failures are ignored.
pub(crate) fn open_url(url: &str) {
    let mut command = if cfg!(target_os = "macos") {
        std::process::Command::new("open")
    } else if cfg!(target_os = "windows") {
        let mut command = std::process::Command::new("cmd");
        command.args(["/C", "start", ""]);
        command
    } else {
        std::process::Command::new("xdg-open")
    };
    let _ = command
        .arg(url)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
}

fn pump_local_worlds(mut worlds: ResMut<LocalWorlds>) {
    worlds.pump();
}

fn pause_on_focus(mut focus: MessageReader<WindowFocused>, mut worlds: ResMut<LocalWorlds>) {
    for message in focus.read() {
        worlds.focus_changed(message.focused);
    }
}

/// Registers [`LocalWorlds`] (inert until attached) and its per-frame systems.
pub(crate) struct LocalWorldsPlugin;

impl Plugin for LocalWorldsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<LocalWorlds>()
            .add_systems(Update, (pump_local_worlds, pause_on_focus));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn focus_only_pauses_while_playing_and_detached_is_inert() {
        let mut worlds = LocalWorlds::default();
        worlds.focus_changed(false);
        worlds.pump();
        worlds.set_playing(true);
        worlds.focus_changed(true);
        worlds.leave_world();
        assert!(!worlds.playing);
    }

    #[test]
    fn input_without_a_client_still_advances_the_model() {
        let mut worlds = LocalWorlds::default();
        worlds.input(Input::BeginCreate);
        assert_eq!(worlds.menu().screen(), Screen::Create);
        worlds.input(Input::Back);
        worlds.input(Input::Refresh);
        assert!(worlds.menu().busy());
    }
}
