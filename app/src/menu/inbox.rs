//! Host adapter for the launcher's optimistic inbox state.
pub(crate) use launcher::menu::inbox::*;

impl super::MenuRuntime {
    /// Applies an inbox action before the account worker collects its queued reports.
    pub(super) fn activate_inbox(&mut self, action: Action) {
        self.feeds.activate_inbox(action);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::menu::{MenuHome, MenuRuntime};
    /// Builds a partial inbox page with a service total larger than its loaded rows.
    fn partial_feed() -> MenuHome {
        MenuHome {
            inbox: vec![super::super::InboxItem {
                instance_id: "old".into(),
                category: "News".into(),
                unread: true,
                ..Default::default()
            }],
            inbox_counts: [(0, 30), (1, 5)].into(),
            inbox_unread: 35,
            ..Default::default()
        }
    }

    #[test]
    fn expired_opened_message_restores_inbox_keyboard_navigation() {
        use super::super::{MenuAction, MenuScreen};
        let mut menu = MenuRuntime::new(true, 2, "Test".into());
        menu.screen = MenuScreen::Inbox;
        menu.feeds.home = partial_feed();
        menu.activate_inbox(Action::Open(0));
        assert_eq!(menu.focus_actions(), [MenuAction::Inbox(Action::Cancel)]);
        menu.feeds.home.inbox.clear();
        menu.feeds.inbox_state.reconcile(&mut menu.feeds.home);
        assert!(menu.feeds.inbox_state.opened.is_none());
        assert!(
            menu.focus_actions()
                .contains(&MenuAction::Inbox(Action::Category(0)))
        );
        assert!(
            menu.focus_actions()
                .contains(&MenuAction::Navigate(MenuScreen::Home))
        );
    }
}
