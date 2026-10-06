//! The host's open Discord join requests: vanilla's popup answers the oldest one over any menu,
//! and the answers wait here for the Discord adapter to send.

use std::{collections::VecDeque, time::Duration};

use launcher::menu::join_requests::JoinRequests;

use super::{MenuAction, MenuRuntime};

#[derive(Debug, Default)]
pub(super) struct JoinRequestUi {
    requests: JoinRequests,
    /// `(user_id, accept)` answers, oldest first.
    replies: VecDeque<(u64, bool)>,
}

impl MenuRuntime {
    /// Queues a request Discord delivered at `now`; true when its user was not already waiting.
    pub(crate) fn push_join_request(&mut self, user_id: u64, name: String, now: Duration) -> bool {
        self.join_requests.requests.push(user_id, name, now)
    }

    /// Drops the requests Discord has closed by `now`.
    pub(crate) fn expire_join_requests(&mut self, now: Duration) {
        self.join_requests.requests.expire(now);
    }

    /// Forgets every request and unsent answer once Discord is gone.
    pub(crate) fn clear_join_requests(&mut self) {
        self.join_requests.requests.clear();
        self.join_requests.replies.clear();
    }

    /// The next answer for Discord.
    pub(crate) fn take_join_reply(&mut self) -> Option<(u64, bool)> {
        self.join_requests.replies.pop_front()
    }

    /// A press on the join request toast opens the pause screen, which shows the popup.
    pub(crate) fn open_join_requests(&mut self) {
        if self.join_requests.requests.current().is_some() {
            self.open_pause();
        }
    }

    /// Who sent the oldest open request, for the view.
    pub(super) fn join_request_view(&self) -> Option<String> {
        self.join_requests
            .requests
            .current()
            .map(|request| request.name.clone())
    }

    /// Whether the popup is up: the menu shows and no other popup outranks it.
    pub(super) fn join_request_prompted(&self) -> bool {
        self.visible
            && self.dialog.is_none()
            && !(self.is_connecting() && self.feeds.server_trust.is_some())
            && self.join_requests.requests.current().is_some()
    }

    pub(super) fn answer_join_request(&mut self, accept: bool) {
        if let Some(reply) = self.join_requests.requests.answer(accept) {
            self.join_requests.replies.push_back(reply);
        }
        self.focused = 0;
    }

    /// Keyboard and gamepad order while the popup is up.
    pub(super) fn join_request_focus_actions(&self) -> Option<Vec<MenuAction>> {
        self.join_request_prompted().then(|| {
            vec![
                MenuAction::JoinRequest(true),
                MenuAction::JoinRequest(false),
            ]
        })
    }
}

#[cfg(test)]
mod tests;
