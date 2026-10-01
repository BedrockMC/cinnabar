use protocol::world_control::{
    Backend, Generator, Prefs, Setup, SetupState, UnavailableReason, World, WorldState, WorldStatus,
};

use super::form::{CreateForm, validate_name};
use super::prompt::{DOCKER_URL, PromptButton, PromptKind};

const FALLBACK_ERROR: &str = "The local world could not be started";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Screen {
    List,
    Create,
    ConfirmDelete,
    Rename,
    /// Waiting for the core to bring the chosen world up.
    Opening,
    /// The user must accept the Minecraft EULA before the server is downloaded.
    Eula,
    /// Vanilla worlds need Docker; see [`WorldsMenu::prompt`].
    BackendPrompt,
    Error,
}

/// User intents; the embedding menu maps its buttons and text fields onto these.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum Input {
    Refresh,
    Select(usize),
    MoveSelection(i32),
    BeginCreate,
    SetName(String),
    SetSeed(String),
    CycleGameMode,
    CycleGenerator,
    CycleDifficulty,
    SubmitCreate,
    RequestDelete,
    ConfirmDelete,
    BeginRename,
    SetRenameText(String),
    SubmitRename,
    Play,
    AcceptEula,
    Prompt(PromptButton),
    /// Cancels the current screen; while opening it also closes the world.
    Back,
}

/// Control-channel work for the executor; each yields one [`Event`].
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum Effect {
    List,
    Create(protocol::world_control::NewWorld),
    Delete(String),
    Rename {
        id: String,
        name: String,
    },
    Open(String),
    /// Waits briefly, then reads the open world's status.
    PollStatus,
    Close,
    /// Focus-driven; its outcome is never surfaced.
    SetPaused(bool),
    LoadPrefs,
    SetPrefs {
        dismiss_docker_prompt: bool,
        redetect: bool,
    },
    AcceptEula,
    /// Handled by the embedding resource, not the control worker.
    OpenUrl(&'static str),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum Event {
    Listed(Vec<World>),
    Created(World),
    Deleted(String),
    Renamed(World),
    Status(WorldStatus),
    Prefs(Prefs, WorldStatus),
    EulaRequired,
    EulaAccepted,
    Failed(String),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Pending {
    Create,
    Play,
}

/// State of the world-management screens; pure, so it runs without a window or core.
#[derive(Debug)]
pub(crate) struct WorldsMenu {
    screen: Screen,
    worlds: Vec<World>,
    selected: Option<usize>,
    create: CreateForm,
    rename_text: String,
    form_error: Option<&'static str>,
    opening: Option<String>,
    ready: Option<String>,
    error: Option<String>,
    busy: bool,
    prefs: Prefs,
    setup: Option<Setup>,
    unavailable: Option<UnavailableReason>,
    prompt_acknowledged: bool,
    pending: Option<Pending>,
    eula_for: Option<String>,
}

impl Default for WorldsMenu {
    fn default() -> Self {
        Self {
            screen: Screen::List,
            worlds: Vec::new(),
            selected: None,
            create: CreateForm::default(),
            rename_text: String::new(),
            form_error: None,
            opening: None,
            ready: None,
            error: None,
            busy: false,
            prefs: Prefs::default(),
            setup: None,
            unavailable: None,
            prompt_acknowledged: false,
            pending: None,
            eula_for: None,
        }
    }
}

impl WorldsMenu {
    pub(crate) fn screen(&self) -> Screen {
        self.screen
    }

    pub(crate) fn worlds(&self) -> &[World] {
        &self.worlds
    }

    pub(crate) fn selected(&self) -> Option<&World> {
        self.selected.and_then(|index| self.worlds.get(index))
    }

    pub(crate) fn selected_index(&self) -> Option<usize> {
        self.selected
    }

    pub(crate) fn create_form(&self) -> &CreateForm {
        &self.create
    }

    pub(crate) fn rename_text(&self) -> &str {
        &self.rename_text
    }

    /// Validation message for the create or rename form, if the last submit failed.
    pub(crate) fn form_error(&self) -> Option<&str> {
        self.form_error
    }

    /// Message for [`Screen::Error`].
    pub(crate) fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    /// Name of the world being opened.
    pub(crate) fn opening_name(&self) -> Option<&str> {
        let id = self.opening.as_deref()?;
        self.worlds
            .iter()
            .find(|w| w.id == id)
            .map(|w| w.name.as_str())
    }

    /// True while a request is in flight; the UI should disable its buttons.
    pub(crate) fn busy(&self) -> bool {
        self.busy
    }

    /// Takes the id of a world that finished opening; the caller then joins the game socket.
    pub(crate) fn take_ready(&mut self) -> Option<String> {
        self.ready.take()
    }

    /// Dedicated-server setup (EULA, download progress) as last reported by the core.
    pub(crate) fn setup(&self) -> Option<&Setup> {
        self.setup.as_ref()
    }

    /// Settings label for the backend new worlds use.
    pub(crate) fn active_backend_label(&self) -> &'static str {
        match self.setup.as_ref().map(|s| s.runtime.as_str()) {
            Some("native") => "Bedrock Dedicated Server",
            Some("container") => "Bedrock Dedicated Server (Docker)",
            _ => "Basic server",
        }
    }

    /// False once the core reports the dedicated server cannot run here.
    fn bds_can_run(&self) -> bool {
        self.setup
            .as_ref()
            .is_none_or(|setup| setup.state != SetupState::Unsupported)
    }

    /// The Docker modal to show, if the current screen is [`Screen::BackendPrompt`].
    pub(crate) fn prompt(&self) -> Option<PromptKind> {
        (self.screen == Screen::BackendPrompt)
            .then(|| self.prompt_kind())
            .flatten()
    }

    fn prompt_kind(&self) -> Option<PromptKind> {
        if self.prompt_acknowledged {
            return None;
        }
        match self.unavailable? {
            UnavailableReason::DockerMissing if !self.prefs.docker_prompt_dismissed => {
                Some(PromptKind::DockerMissing)
            }
            UnavailableReason::DockerNotRunning => Some(PromptKind::DockerNotRunning),
            _ => None,
        }
    }

    fn note_status(&mut self, status: &WorldStatus) {
        self.unavailable = status.backend_unavailable_reason;
        if status.setup.is_some() {
            self.setup.clone_from(&status.setup);
        }
    }

    /// Runs `pending` now, or parks it behind the Docker modal.
    fn gate(&mut self, pending: Pending) -> Vec<Effect> {
        let needs_docker = match pending {
            Pending::Create => true,
            Pending::Play => self.selected().is_some_and(|w| w.backend == Backend::Bds),
        };
        if needs_docker && self.prompt_kind().is_some() {
            self.pending = Some(pending);
            self.screen = Screen::BackendPrompt;
            return Vec::new();
        }
        self.proceed(pending)
    }

    fn proceed(&mut self, pending: Pending) -> Vec<Effect> {
        match pending {
            Pending::Create => {
                self.create = CreateForm::default();
                if !self.bds_can_run() {
                    // Default terrain is BDS-only; superflat is all the basic server hosts.
                    self.create.generator = Generator::Flat;
                }
                self.form_error = None;
                self.screen = Screen::Create;
                Vec::new()
            }
            Pending::Play => {
                let Some(id) = self.selected().map(|w| w.id.clone()) else {
                    self.screen = Screen::List;
                    return Vec::new();
                };
                self.opening = Some(id.clone());
                self.ready = None;
                self.screen = Screen::Opening;
                vec![Effect::Open(id)]
            }
        }
    }

    fn resume_pending(&mut self) -> Vec<Effect> {
        match self.pending.take() {
            Some(pending) => self.proceed(pending),
            None => {
                self.screen = Screen::List;
                Vec::new()
            }
        }
    }

    fn prompt_button(&mut self, button: PromptButton) -> Vec<Effect> {
        if self.screen != Screen::BackendPrompt {
            return Vec::new();
        }
        match button {
            PromptButton::PlayAnyway => {
                self.prompt_acknowledged = true;
                self.resume_pending()
            }
            PromptButton::GetDocker => vec![Effect::OpenUrl(DOCKER_URL)],
            PromptButton::DontShowAgain => {
                self.prefs.docker_prompt_dismissed = true;
                let mut effects = vec![Effect::SetPrefs {
                    dismiss_docker_prompt: true,
                    redetect: false,
                }];
                effects.extend(self.resume_pending());
                effects
            }
            PromptButton::Retry => {
                self.busy = true;
                vec![Effect::SetPrefs {
                    dismiss_docker_prompt: false,
                    redetect: true,
                }]
            }
        }
    }

    fn select_clamped(&mut self, index: Option<usize>) {
        self.selected = index
            .filter(|_| !self.worlds.is_empty())
            .map(|i| i.min(self.worlds.len() - 1));
    }

    fn fail(&mut self, message: impl Into<String>) {
        self.error = Some(message.into());
        self.screen = Screen::Error;
        self.busy = false;
    }

    pub(crate) fn update(&mut self, input: Input) -> Vec<Effect> {
        if self.busy && !matches!(input, Input::Back) {
            return Vec::new();
        }
        match input {
            Input::Refresh => {
                self.busy = true;
                vec![Effect::List]
            }
            Input::Select(index) => {
                if self.screen == Screen::List && index < self.worlds.len() {
                    self.selected = Some(index);
                }
                Vec::new()
            }
            Input::MoveSelection(delta) => {
                if self.screen == Screen::List && !self.worlds.is_empty() {
                    let last = self.worlds.len() - 1;
                    let next = self
                        .selected
                        .map_or(0, |i| i.saturating_add_signed(delta as isize).min(last));
                    self.selected = Some(next);
                }
                Vec::new()
            }
            Input::BeginCreate => {
                if self.screen == Screen::List {
                    return self.gate(Pending::Create);
                }
                Vec::new()
            }
            Input::SetName(name) => {
                self.create.name = name;
                Vec::new()
            }
            Input::SetSeed(seed) => {
                self.create.seed_text = seed;
                Vec::new()
            }
            Input::CycleGameMode => {
                self.create.cycle_game_mode();
                Vec::new()
            }
            Input::CycleGenerator => {
                self.create.cycle_generator();
                Vec::new()
            }
            Input::CycleDifficulty => {
                self.create.cycle_difficulty();
                Vec::new()
            }
            Input::SubmitCreate => {
                if self.screen != Screen::Create {
                    return Vec::new();
                }
                match self.create.build() {
                    Ok(new_world) => {
                        self.form_error = None;
                        self.busy = true;
                        vec![Effect::Create(new_world)]
                    }
                    Err(error) => {
                        self.form_error = Some(error.message());
                        Vec::new()
                    }
                }
            }
            Input::RequestDelete => {
                if self.screen == Screen::List && self.selected().is_some() {
                    self.screen = Screen::ConfirmDelete;
                }
                Vec::new()
            }
            Input::ConfirmDelete => {
                let Some(id) = self.selected().map(|w| w.id.clone()) else {
                    return Vec::new();
                };
                if self.screen != Screen::ConfirmDelete {
                    return Vec::new();
                }
                self.busy = true;
                vec![Effect::Delete(id)]
            }
            Input::BeginRename => {
                if let (Screen::List, Some(world)) = (self.screen, self.selected()) {
                    self.rename_text = world.name.clone();
                    self.form_error = None;
                    self.screen = Screen::Rename;
                }
                Vec::new()
            }
            Input::SetRenameText(text) => {
                self.rename_text = text;
                Vec::new()
            }
            Input::SubmitRename => {
                let Some(id) = self.selected().map(|w| w.id.clone()) else {
                    return Vec::new();
                };
                if self.screen != Screen::Rename {
                    return Vec::new();
                }
                match validate_name(&self.rename_text) {
                    Ok(name) => {
                        self.form_error = None;
                        self.busy = true;
                        vec![Effect::Rename { id, name }]
                    }
                    Err(error) => {
                        self.form_error = Some(error.message());
                        Vec::new()
                    }
                }
            }
            Input::Play => {
                if self.screen != Screen::List || self.selected().is_none() {
                    return Vec::new();
                }
                self.gate(Pending::Play)
            }
            Input::AcceptEula => {
                if self.screen != Screen::Eula {
                    return Vec::new();
                }
                self.busy = true;
                vec![Effect::AcceptEula]
            }
            Input::Prompt(button) => self.prompt_button(button),
            Input::Back => self.back(),
        }
    }

    fn back(&mut self) -> Vec<Effect> {
        match self.screen {
            Screen::Opening => {
                self.opening = None;
                self.busy = false;
                self.screen = Screen::List;
                vec![Effect::Close]
            }
            Screen::Create
            | Screen::ConfirmDelete
            | Screen::Rename
            | Screen::Error
            | Screen::Eula
            | Screen::BackendPrompt => {
                self.form_error = None;
                self.error = None;
                self.pending = None;
                self.eula_for = None;
                self.busy = false;
                self.screen = Screen::List;
                Vec::new()
            }
            Screen::List => Vec::new(),
        }
    }

    pub(crate) fn apply(&mut self, event: Event) -> Vec<Effect> {
        match event {
            Event::Listed(worlds) => {
                let keep = self.selected().map(|w| w.id.clone());
                self.worlds = worlds;
                let index = keep
                    .and_then(|id| self.worlds.iter().position(|w| w.id == id))
                    .or(Some(0));
                self.select_clamped(index);
                self.busy = false;
                Vec::new()
            }
            Event::Created(world) => {
                self.worlds.insert(0, world);
                self.selected = Some(0);
                self.busy = false;
                self.screen = Screen::List;
                Vec::new()
            }
            Event::Deleted(id) => {
                self.worlds.retain(|w| w.id != id);
                let index = self.selected;
                self.select_clamped(index);
                self.busy = false;
                self.screen = Screen::List;
                Vec::new()
            }
            Event::Renamed(world) => {
                if let Some(slot) = self.worlds.iter_mut().find(|w| w.id == world.id) {
                    *slot = world;
                }
                self.busy = false;
                self.screen = Screen::List;
                Vec::new()
            }
            Event::Status(status) => {
                self.note_status(&status);
                self.apply_status(&status)
            }
            Event::Prefs(prefs, status) => {
                self.prefs = prefs;
                self.note_status(&status);
                self.busy = false;
                if self.screen == Screen::BackendPrompt && self.prompt_kind().is_none() {
                    return self.resume_pending();
                }
                Vec::new()
            }
            Event::EulaRequired => {
                self.eula_for = self.opening.take();
                self.busy = false;
                self.screen = Screen::Eula;
                Vec::new()
            }
            Event::EulaAccepted => {
                self.busy = false;
                let Some(id) = self.eula_for.take() else {
                    self.screen = Screen::List;
                    return Vec::new();
                };
                self.opening = Some(id.clone());
                self.screen = Screen::Opening;
                vec![Effect::Open(id)]
            }
            Event::Failed(message) => {
                self.opening = None;
                self.fail(message);
                Vec::new()
            }
        }
    }

    fn apply_status(&mut self, status: &WorldStatus) -> Vec<Effect> {
        let Some(opening) = self.opening.clone() else {
            return Vec::new();
        };
        if status.world_id.as_deref().is_some_and(|id| id != opening) {
            return Vec::new();
        }
        match status.state {
            WorldState::Running => {
                self.opening = None;
                self.ready = Some(opening);
                self.screen = Screen::List;
                Vec::new()
            }
            WorldState::Starting | WorldState::Stopping => vec![Effect::PollStatus],
            WorldState::Idle | WorldState::Failed => {
                self.opening = None;
                self.fail(
                    status
                        .error
                        .clone()
                        .unwrap_or_else(|| FALLBACK_ERROR.to_owned()),
                );
                vec![Effect::Close]
            }
        }
    }
}

#[cfg(test)]
mod tests;
