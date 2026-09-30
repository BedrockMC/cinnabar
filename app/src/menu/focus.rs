//! Keyboard and gamepad focus: the actions each launcher screen cycles
//! through, and moving or activating the focused one.

use super::*;

impl MenuRuntime {
    pub(crate) fn move_focus(&mut self, direction: i32) {
        let actions = self.focus_actions();
        if actions.is_empty() {
            self.focused = 0;
            return;
        }
        let length = actions.len() as i32;
        self.focused = (self.focused as i32 + direction).rem_euclid(length) as usize;
        match actions[self.focused] {
            MenuAction::AddName => self.focus_field(MenuField::Name),
            MenuAction::AddAddress => self.focus_field(MenuField::Address),
            _ => {
                self.field = None;
                self.text_selected = false;
            }
        }
    }

    pub(crate) fn activate_focused(&mut self) {
        let Some(action) = self.focus_actions().get(self.focused).copied() else {
            return;
        };
        self.activate(action);
    }

    /// The actions keyboard and gamepad focus cycles through on the current screen.
    pub(super) fn focus_actions(&self) -> Vec<MenuAction> {
        if let Some(dialog) = self.dialog {
            return match dialog {
                MenuDialog::Exit => vec![MenuAction::ConfirmExit, MenuAction::DismissDialog],
                MenuDialog::RemoveSaved(index) => vec![
                    MenuAction::ConfirmRemoveSaved(index),
                    MenuAction::DismissDialog,
                ],
            };
        }
        let nav = || {
            vec![
                MenuAction::Navigate(MenuScreen::Home),
                MenuAction::Navigate(MenuScreen::Play),
                MenuAction::Navigate(MenuScreen::Social),
                MenuAction::Navigate(MenuScreen::Servers),
                MenuAction::Navigate(MenuScreen::Profile),
                MenuAction::Navigate(MenuScreen::Settings),
                MenuAction::OpenExitDialog,
            ]
        };
        match self.screen {
            MenuScreen::Home => {
                let mut actions = nav();
                actions.extend((0..self.friends.len().min(1)).map(MenuAction::PlayFriend));
                actions.extend((0..self.realms.len().min(1)).map(MenuAction::PlayRealm));
                actions.extend((0..self.featured.len().min(2)).map(MenuAction::PlayFeatured));
                actions
            }
            MenuScreen::Play => {
                let mut actions = nav();
                actions.extend((0..self.friends.len()).map(MenuAction::PlayFriend));
                actions.extend((0..self.realms.len()).map(MenuAction::PlayRealm));
                actions.extend(
                    self.servers
                        .iter()
                        .enumerate()
                        .filter(|(_, server)| server.last_joined_unix > 0)
                        .map(|(index, _)| MenuAction::PlaySaved(index)),
                );
                actions
            }
            MenuScreen::Social => {
                let mut actions = nav();
                actions.push(MenuAction::RefreshCatalog);
                actions.extend((0..self.friends.len()).map(MenuAction::PlayFriend));
                actions
            }
            MenuScreen::Servers => {
                let mut actions = nav();
                actions.extend([
                    MenuAction::SelectServerTab(MenuServerTab::Featured),
                    MenuAction::SelectServerTab(MenuServerTab::Favorites),
                    MenuAction::SelectServerTab(MenuServerTab::Recent),
                    MenuAction::SelectServerTab(MenuServerTab::Saved),
                    MenuAction::PlayAddServer,
                ]);
                match self.server_tab {
                    MenuServerTab::Featured => {
                        actions.extend((0..self.featured.len()).map(MenuAction::PlayFeatured));
                        actions.extend((0..self.gatherings.len()).map(MenuAction::PlayGathering));
                    }
                    MenuServerTab::Favorites => actions.extend(
                        self.servers
                            .iter()
                            .enumerate()
                            .filter(|(_, server)| server.favorite)
                            .map(|(index, _)| MenuAction::PlaySaved(index)),
                    ),
                    MenuServerTab::Recent => actions.extend(
                        self.servers
                            .iter()
                            .enumerate()
                            .filter(|(_, server)| server.last_joined_unix > 0)
                            .map(|(index, _)| MenuAction::PlaySaved(index)),
                    ),
                    MenuServerTab::Saved => {
                        actions.extend((0..self.servers.len()).map(MenuAction::PlaySaved));
                    }
                }
                actions
            }
            MenuScreen::Profile => {
                let mut actions = nav();
                actions.push(
                    if matches!(
                        self.auth_process.as_ref().map(AuthSupervisor::state),
                        Some(AuthState::Checking | AuthState::AwaitingCode { .. })
                    ) {
                        MenuAction::CancelSignIn
                    } else {
                        MenuAction::StartSignIn
                    },
                );
                actions
            }
            MenuScreen::Settings => {
                let mut actions = nav();
                actions.extend([
                    MenuAction::SettingsScale(1),
                    MenuAction::SettingsScale(2),
                    MenuAction::SettingsScale(3),
                    MenuAction::SettingsScale(4),
                ]);
                actions
            }
            MenuScreen::AddServer => vec![
                MenuAction::AddName,
                MenuAction::AddAddress,
                MenuAction::AddSave,
                MenuAction::AddSaveConnect,
                MenuAction::AddBack,
            ],
            MenuScreen::Pause => vec![
                MenuAction::PauseResume,
                MenuAction::PauseSettings,
                MenuAction::PauseDisconnect,
            ],
            MenuScreen::Death => vec![MenuAction::Respawn, MenuAction::Navigate(MenuScreen::Pause)],
            MenuScreen::Inbox | MenuScreen::Friends => vec![MenuAction::Navigate(MenuScreen::Home)],
            MenuScreen::Store => vec![MenuAction::Store(crate::store::StoreAction::Back)],
        }
    }
}
