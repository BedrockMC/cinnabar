use protocol::world_control::{World, WorldState, WorldStatus};

use super::form::{CreateForm, validate_name};

const FALLBACK_ERROR: &str = "The local world could not be started";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Screen {
    List,
    Create,
    ConfirmDelete,
    Rename,
    /// Waiting for the core to bring the chosen world up.
    Opening,
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
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum Event {
    Listed(Vec<World>),
    Created(World),
    Deleted(String),
    Renamed(World),
    Status(WorldStatus),
    Failed(String),
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
                    self.create = CreateForm::default();
                    self.form_error = None;
                    self.screen = Screen::Create;
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
                let Some(id) = self.selected().map(|w| w.id.clone()) else {
                    return Vec::new();
                };
                if self.screen != Screen::List {
                    return Vec::new();
                }
                self.opening = Some(id.clone());
                self.ready = None;
                self.screen = Screen::Opening;
                vec![Effect::Open(id)]
            }
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
            Screen::Create | Screen::ConfirmDelete | Screen::Rename | Screen::Error => {
                self.form_error = None;
                self.error = None;
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
            Event::Status(status) => self.apply_status(&status),
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
