//! The menu's view of the core's account control surface. The core-relay lane
//! adds typed clients for these control methods and events; implementing
//! [`AccountControl`] over them links the play and sign-in screens without the
//! menu knowing the transport. Until one is supplied, the account catalog and
//! the auth supervisor keep feeding the menu as before.
#![cfg_attr(
    not(test),
    allow(dead_code, reason = "fed by the core-relay control clients")
)]

use super::{AuthState, MenuFriendCard, MenuRealmCard, MenuRuntime};

/// Control method names the implementation calls.
#[allow(dead_code, reason = "named for the core-relay control clients")]
pub(crate) mod method {
    pub(crate) const REALMS_LIST: &str = "realms_list.v1";
    pub(crate) const FRIENDS_LIST: &str = "friends_list.v1";
    pub(crate) const CONNECT: &str = "connect.v1";
    pub(crate) const ACCOUNT_STATUS: &str = "account_status.v1";
    pub(crate) const SIGN_OUT: &str = "sign_out.v1";
}

/// An account-surface event pushed by the core.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum AccountEvent {
    /// The sign-in state changed (device code shown, signed in, signed out).
    Auth(AuthState),
    /// The live session ended; the reason shows on the disconnect screen.
    Disconnected { reason: String },
}

/// What the play and sign-in screens need from the core.
pub(crate) trait AccountControl {
    /// `account_status.v1`: the current sign-in state, when known.
    fn account_status(&mut self) -> Option<AuthState>;
    /// `realms_list.v1`: joinable realms, or `None` while unavailable.
    fn realms(&mut self) -> Option<Vec<MenuRealmCard>>;
    /// `friends_list.v1`: friend worlds, or `None` while unavailable.
    fn friends(&mut self) -> Option<Vec<MenuFriendCard>>;
    /// `sign_out.v1`; `true` once the core accepted it.
    fn sign_out(&mut self) -> bool;
    /// The next pending account event, if any.
    fn poll_event(&mut self) -> Option<AccountEvent>;
}

impl MenuRuntime {
    /// Pull the core's account state into the menu: lists replace the catalog's,
    /// the status overrides the auth supervisor's, events surface on screen, and
    /// a pending sign-out request is sent.
    pub(crate) fn sync_account_control(&mut self, control: &mut dyn AccountControl) {
        if let Some(realms) = control.realms() {
            self.realms = realms;
        }
        if let Some(friends) = control.friends() {
            self.friends = friends;
        }
        if let Some(status) = control.account_status() {
            self.control_auth = Some(status);
        }
        while let Some(event) = control.poll_event() {
            match event {
                AccountEvent::Auth(state) => self.control_auth = Some(state),
                AccountEvent::Disconnected { reason } => {
                    self.disconnect_message = Some(reason);
                }
            }
        }
        if std::mem::take(&mut self.sign_out_requested) && control.sign_out() {
            self.control_auth = Some(AuthState::SignedOut);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fake {
        events: Vec<AccountEvent>,
        signed_out: bool,
    }

    impl AccountControl for Fake {
        fn account_status(&mut self) -> Option<AuthState> {
            Some(AuthState::Authenticated)
        }
        fn realms(&mut self) -> Option<Vec<MenuRealmCard>> {
            None
        }
        fn friends(&mut self) -> Option<Vec<MenuFriendCard>> {
            Some(vec![MenuFriendCard {
                gamertag: "Alex".into(),
                world_name: "Base".into(),
                members: "1 players".into(),
                xuid: "1".into(),
            }])
        }
        fn sign_out(&mut self) -> bool {
            self.signed_out = true;
            true
        }
        fn poll_event(&mut self) -> Option<AccountEvent> {
            self.events.pop()
        }
    }

    #[test]
    fn control_state_feeds_the_menu_view() {
        assert_eq!(method::SIGN_OUT, "sign_out.v1");
        let mut menu = MenuRuntime::new(true, 2, "Steve".to_owned());
        let mut control = Fake {
            events: vec![AccountEvent::Disconnected {
                reason: "Server closed".into(),
            }],
            signed_out: false,
        };
        menu.sync_account_control(&mut control);
        let view = menu.view();
        assert_eq!(view.auth_state, AuthState::Authenticated);
        assert_eq!(view.friends.len(), 1);
        assert_eq!(view.disconnect_message.as_deref(), Some("Server closed"));
        menu.activate(super::super::MenuAction::SignOut);
        menu.sync_account_control(&mut control);
        assert!(control.signed_out);
        assert_eq!(menu.view().auth_state, AuthState::SignedOut);
    }
}
