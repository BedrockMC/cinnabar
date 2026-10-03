//! The play screen's worlds tab over the local-worlds module: its list feeds
//! the tab's cards, the create/edit/template screens and their modals relay
//! presses to the module, an opened world is joined through the launcher core
//! and closed when its session ends, and the pause menu pauses it.

use protocol::world_control::{Difficulty, GameMode, World};

use super::{LocalWorldCard, MenuAction, MenuField, MenuRuntime, MenuScreen, PendingConnect};
use crate::local_worlds::{
    Input, LocalWorlds, Progress, PromptButton, Screen, Tab, WorldsView, game_mode_label,
    world_type_label,
};

/// A press on a local-world screen or modal; the menu forwards it to the module.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum LocalWorldAction {
    Edit(usize),
    BeginCreate,
    OpenTemplates,
    Back,
    Tab(Tab),
    /// Focuses the world name field (create or edit).
    NameField,
    SeedField,
    GameMode(GameMode),
    Difficulty(Difficulty),
    Flat(bool),
    Create,
    Save,
    Discard,
    PlayFromEdit,
    Delete,
    ConfirmDelete,
    AcceptEula,
    ViewEula,
    Prompt(PromptButton),
}

impl LocalWorldAction {
    /// The text field this press focuses, if any.
    pub(super) fn field(self) -> Option<MenuField> {
        match self {
            Self::NameField => Some(MenuField::WorldName),
            Self::SeedField => Some(MenuField::WorldSeed),
            _ => None,
        }
    }

    fn input(self) -> Option<Input> {
        Some(match self {
            Self::Edit(index) => Input::BeginEdit(index),
            Self::BeginCreate => Input::BeginCreate,
            Self::OpenTemplates => Input::OpenTemplates,
            Self::Back => Input::Back,
            Self::Tab(tab) => Input::SelectTab(tab),
            Self::NameField | Self::SeedField => return None,
            Self::GameMode(mode) => Input::SetGameMode(mode),
            Self::Difficulty(difficulty) => Input::SetDifficulty(difficulty),
            Self::Flat(flat) => Input::SetFlat(flat),
            Self::Create => Input::SubmitCreate,
            Self::Save => Input::SubmitEdit,
            Self::Discard => Input::DiscardEdit,
            Self::PlayFromEdit => Input::PlayFromEdit,
            Self::Delete => Input::RequestDelete,
            Self::ConfirmDelete => Input::ConfirmDelete,
            Self::AcceptEula => Input::AcceptEula,
            Self::ViewEula => Input::OpenEulaLink,
            Self::Prompt(button) => button.input(),
        })
    }
}

/// The menu's side of the local-world screens: the mirrored view, queued presses and the
/// name and seed fields' editors.
#[derive(Debug)]
pub(super) struct LocalWorldsUi {
    view: WorldsView,
    actions: Vec<LocalWorldAction>,
    pub(super) name: ui::ChatEditor,
    pub(super) seed: ui::ChatEditor,
    /// The world being joined, for the loading screen's connect stage.
    joining: Option<String>,
}

impl Default for LocalWorldsUi {
    fn default() -> Self {
        Self {
            view: WorldsView::default(),
            actions: Vec::new(),
            name: super::input::field_editor(MenuField::WorldName),
            seed: super::input::field_editor(MenuField::WorldSeed),
            joining: None,
        }
    }
}

impl MenuRuntime {
    /// Mirror the module's worlds and screens, forward presses and typed text, join a world
    /// that finished opening, and track whether a local-world session is live.
    pub(crate) fn sync_local_worlds(&mut self, worlds: &mut LocalWorlds, in_session: bool) {
        self.sync_storage_worlds(worlds);
        self.push_local_text(worlds);
        for action in std::mem::take(&mut self.local_ui.actions) {
            if let Some(input) = action.input() {
                worlds.input(input);
            }
        }
        let before = self.local_ui.view.screen;
        let mut view = worlds.menu().view();
        if view.screen != before {
            self.load_local_text(&view);
        }
        let cards = worlds.menu().worlds().iter().map(world_card).collect();
        self.set_local_worlds(cards);
        if let Some(index) = self.take_local_world_request() {
            worlds.input(Input::Select(index));
            worlds.input(Input::Play);
            view = worlds.menu().view();
        }
        if let Some(id) = worlds.take_ready() {
            let name = worlds
                .menu()
                .worlds()
                .iter()
                .find(|world| world.id == id)
                .map_or(id, |world| world.name.clone());
            self.request_local_world_join(name);
        }
        if self.local_world_joined && self.connecting {
            let name = self.local_ui.joining.as_deref().unwrap_or_default();
            view.progress = Some(Progress::connecting(name));
        }
        self.finish_storage_world(view.screen);
        self.local_ui.view = view;
        let active = self.local_world_joined && (in_session || self.connecting);
        if active != self.local_world_active {
            self.local_world_active = active;
            if active {
                worlds.set_playing(true);
            } else {
                // The core saves and stops the world once its session is over.
                worlds.leave_world();
                self.local_world_joined = false;
                self.local_ui.joining = None;
            }
        }
        worlds.set_pause_menu(active && self.visible && self.screen == MenuScreen::Pause);
    }

    /// The local-world screens' state for the menu view, with the fields' live text.
    pub(super) fn local_view(&self) -> WorldsView {
        let mut view = self.local_ui.view.clone();
        match view.screen {
            Screen::Create => {
                self.local_ui
                    .name
                    .as_str()
                    .clone_into(&mut view.create.name);
                self.local_ui
                    .seed
                    .as_str()
                    .clone_into(&mut view.create.seed_text);
            }
            Screen::Edit => {
                if let Some(edit) = &mut view.edit {
                    self.local_ui.name.as_str().clone_into(&mut edit.name);
                }
            }
            _ => {}
        }
        view
    }

    /// Whether a local-world screen covers the worlds tab (Escape then backs out of it).
    pub(super) fn local_screen_open(&self) -> bool {
        matches!(
            self.screen,
            MenuScreen::Play | MenuScreen::Social | MenuScreen::Servers
        ) && !matches!(self.local_ui.view.screen, Screen::List | Screen::Opening)
    }

    pub(super) fn queue_local_action(&mut self, action: LocalWorldAction) {
        self.local_ui.actions.push(action);
    }

    fn push_local_text(&mut self, worlds: &mut LocalWorlds) {
        let ui = &self.local_ui;
        let view = &ui.view;
        match view.screen {
            Screen::Create
                if ui.name.as_str() != view.create.name
                    || ui.seed.as_str() != view.create.seed_text =>
            {
                worlds.input(Input::SetName(ui.name.as_str().to_owned()));
                worlds.input(Input::SetSeed(ui.seed.as_str().to_owned()));
            }
            Screen::Edit
                if view
                    .edit
                    .as_ref()
                    .is_some_and(|edit| edit.name != ui.name.as_str()) =>
            {
                worlds.input(Input::SetEditName(ui.name.as_str().to_owned()));
            }
            _ => {}
        }
    }

    /// Entering a form loads its text; leaving the forms drops their field focus.
    fn load_local_text(&mut self, view: &WorldsView) {
        match view.screen {
            Screen::Create => {
                self.local_ui.name.set_text(&view.create.name);
                self.local_ui.seed.set_text(&view.create.seed_text);
            }
            Screen::Edit => {
                if let Some(edit) = &view.edit {
                    self.local_ui.name.set_text(&edit.name);
                }
            }
            _ => {}
        }
        if !matches!(view.screen, Screen::Create | Screen::Edit)
            && matches!(
                self.field,
                Some(MenuField::WorldName | MenuField::WorldSeed)
            )
        {
            self.field = None;
        }
    }

    /// Keyboard and gamepad focus order on the local-world screens.
    pub(super) fn local_focus_actions(&self) -> Option<Vec<MenuAction>> {
        use LocalWorldAction as A;
        let view = &self.local_ui.view;
        let local = |actions: &[LocalWorldAction]| {
            Some(
                actions
                    .iter()
                    .copied()
                    .map(MenuAction::LocalWorld)
                    .collect(),
            )
        };
        if let Some(prompt) = view.prompt {
            return local(
                &prompt
                    .buttons()
                    .iter()
                    .map(|button| A::Prompt(*button))
                    .collect::<Vec<_>>(),
            );
        }
        match view.screen {
            Screen::List | Screen::Opening | Screen::BackendPrompt => None,
            Screen::Create => match view.tab {
                Tab::General => local(&[
                    A::Create,
                    A::Tab(Tab::Advanced),
                    A::NameField,
                    A::GameMode(GameMode::Survival),
                    A::GameMode(GameMode::Creative),
                    A::Difficulty(Difficulty::Peaceful),
                    A::Difficulty(Difficulty::Easy),
                    A::Difficulty(Difficulty::Normal),
                    A::Difficulty(Difficulty::Hard),
                ]),
                Tab::Advanced => local(&[
                    A::Create,
                    A::Tab(Tab::General),
                    A::SeedField,
                    A::Flat(false),
                    A::Flat(true),
                ]),
            },
            Screen::Edit => local(&[
                A::PlayFromEdit,
                A::NameField,
                A::GameMode(GameMode::Survival),
                A::GameMode(GameMode::Creative),
                A::GameMode(GameMode::Adventure),
                A::Difficulty(Difficulty::Peaceful),
                A::Difficulty(Difficulty::Easy),
                A::Difficulty(Difficulty::Normal),
                A::Difficulty(Difficulty::Hard),
                A::Delete,
            ]),
            Screen::Templates => Some(vec![
                MenuAction::LocalWorld(A::BeginCreate),
                MenuAction::Store(crate::store::OPEN),
            ]),
            Screen::ConfirmDelete => local(&[A::Back, A::ConfirmDelete]),
            Screen::ConfirmLeaveEdit => local(&[A::Save, A::Discard]),
            Screen::Eula => local(&[A::AcceptEula, A::ViewEula, A::Back]),
            Screen::Error => local(&[A::Back]),
        }
    }

    fn request_local_world_join(&mut self, name: String) {
        self.begin_fresh_transfer_chain();
        self.stop_catalog();
        self.local_world_joined = true;
        self.local_ui.joining = Some(name.clone());
        self.pending_connect = Some(PendingConnect {
            address: name,
            auth_cache: None,
            local_world: true,
        });
        self.mark_connecting();
    }
}

/// Presents metadata from the core catalog consistently in Play and Storage.
pub(super) fn world_card(world: &World) -> LocalWorldCard {
    LocalWorldCard {
        name: world.name.clone(),
        game_mode: game_mode_label(world.game_mode).to_owned(),
        world_type: world_type_label(world.generator).to_owned(),
        date: civil_date(world.last_played_unix.max(world.created_unix)),
        size: file_size(world.size_bytes),
    }
}

/// A world's size as the Worlds tab captions it: one decimal in KB, MB or GB.
pub(crate) fn file_size(bytes: u64) -> String {
    const UNITS: [&str; 3] = ["KB", "MB", "GB"];
    let mut value = bytes as f64 / 1024.0;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    format!("{value:.1} {}", UNITS[unit])
}

/// `month/day/year` of a UTC unix time; empty before the epoch.
pub(crate) fn civil_date(unix: i64) -> String {
    if unix <= 0 {
        return String::new();
    }
    // Days-to-civil over 400-year eras (proleptic Gregorian).
    let days = unix / 86_400 + 719_468;
    let era = days / 146_097;
    let day_of_era = days - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let shifted_month = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * shifted_month + 2) / 5 + 1;
    let month = if shifted_month < 10 {
        shifted_month + 3
    } else {
        shifted_month - 9
    };
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    format!("{month}/{day}/{year}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates_render_as_month_day_year() {
        assert_eq!(civil_date(0), "");
        assert_eq!(civil_date(951_782_400), "2/29/2000");
        assert_eq!(civil_date(1_790_553_600), "9/28/2026");
    }

    #[test]
    fn sizes_render_in_the_largest_whole_unit() {
        assert_eq!(file_size(0), "0.0 KB");
        assert_eq!(file_size(512 * 1024), "512.0 KB");
        assert_eq!(file_size(5 * 1024 * 1024 + 300 * 1024), "5.3 MB");
        assert_eq!(file_size(3 * 1024 * 1024 * 1024), "3.0 GB");
    }

    #[test]
    fn a_chosen_card_selects_the_world_in_the_module() {
        let mut menu = MenuRuntime::new(true, 2, "Steve".to_owned());
        let mut worlds = LocalWorlds::default();
        menu.sync_local_worlds(&mut worlds, false);
        assert!(menu.view().local_worlds.is_empty());
        menu.activate(MenuAction::PlayLocalWorld(0));
        assert_eq!(menu.take_local_world_request(), None);
    }

    #[test]
    fn a_local_world_session_closes_the_world_when_it_ends() {
        let mut menu = MenuRuntime::new(true, 2, "Steve".to_owned());
        let mut worlds = LocalWorlds::default();
        menu.request_local_world_join("Home".to_owned());
        assert!(
            menu.pending_connect
                .as_ref()
                .is_some_and(|join| join.local_world)
        );
        menu.sync_local_worlds(&mut worlds, false);
        assert!(menu.local_world_active, "connecting counts as live");
        assert_eq!(
            menu.view().local.progress,
            Some(Progress::connecting("Home")),
            "the loading screen's last stage is the join"
        );
        menu.connecting = false;
        menu.sync_local_worlds(&mut worlds, false);
        assert!(!menu.local_world_active && !menu.local_world_joined);
    }

    /// Create-screen presses reach the module, and typed text lands in its form before submit.
    #[test]
    fn create_screen_presses_and_text_reach_the_module() {
        let mut menu = MenuRuntime::new(true, 2, "Steve".to_owned());
        let mut worlds = LocalWorlds::default();
        menu.activate(MenuAction::Navigate(MenuScreen::Play));
        menu.activate(MenuAction::LocalWorld(LocalWorldAction::BeginCreate));
        menu.sync_local_worlds(&mut worlds, false);
        assert_eq!(menu.view().local.screen, Screen::Create);
        assert_eq!(
            menu.local_ui.name.as_str(),
            "My World",
            "the form's text loads"
        );
        menu.activate(MenuAction::LocalWorld(LocalWorldAction::NameField));
        assert_eq!(menu.field, Some(MenuField::WorldName));
        menu.local_ui.name.set_text("Castle");
        menu.activate(MenuAction::LocalWorld(LocalWorldAction::GameMode(
            GameMode::Creative,
        )));
        menu.sync_local_worlds(&mut worlds, false);
        let form = worlds.menu().create_form();
        assert_eq!(
            (form.name.as_str(), form.game_mode),
            ("Castle", GameMode::Creative)
        );
        assert!(menu.local_screen_open());
        menu.go_back();
        menu.sync_local_worlds(&mut worlds, false);
        assert_eq!(menu.view().local.screen, Screen::List);
        assert_eq!(menu.field, None, "leaving the form drops its field");
    }
}
