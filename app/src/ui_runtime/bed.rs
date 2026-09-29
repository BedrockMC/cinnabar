//! Bed screen state: sleeping opens the chat-style screen and a wake request leaves the bed.

use super::UiRuntime;

impl UiRuntime {
    /// Opens the chat-style bed screen on falling asleep and closes it on waking.
    pub(crate) fn set_local_sleeping(&mut self, sleeping: bool) {
        if sleeping == self.local_sleeping {
            return;
        }
        self.local_sleeping = sleeping;
        if sleeping {
            if !self.inventory_open && !self.forms.owns_input() {
                self.open_chat();
            }
        } else {
            self.wake_requested = false;
            if self.chat_focused {
                self.close_chat();
            }
        }
    }

    /// Queues one StopSleeping action; a no-op unless the local player is asleep.
    pub(crate) fn request_wake(&mut self) {
        self.wake_requested |= self.local_sleeping;
    }

    pub(crate) fn take_wake_request(&mut self) -> bool {
        std::mem::take(&mut self.wake_requested)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sleeping_opens_chat_and_wake_request_is_taken_once() {
        let mut runtime = UiRuntime::new(1);
        runtime.request_wake();
        assert!(!runtime.take_wake_request());

        runtime.set_local_sleeping(true);
        assert!(runtime.chat_focused());
        runtime.request_wake();
        assert!(runtime.take_wake_request());
        assert!(!runtime.take_wake_request());

        runtime.request_wake();
        runtime.set_local_sleeping(false);
        assert!(!runtime.chat_focused());
        assert!(!runtime.take_wake_request());
    }

    #[test]
    fn escape_closed_bed_screen_does_not_reopen_while_still_asleep() {
        let mut runtime = UiRuntime::new(1);
        runtime.set_local_sleeping(true);
        runtime.close_chat();
        runtime.set_local_sleeping(true);
        assert!(!runtime.chat_focused());
    }
}
