//! Bed screen state: sleeping shows the bed screen and a wake request leaves the bed.

use super::UiRuntime;

impl UiRuntime {
    /// Tracks the local player's sleep; waking closes a chat opened from the bed.
    pub(crate) fn set_local_sleeping(&mut self, sleeping: bool) {
        if sleeping == self.local_sleeping {
            return;
        }
        self.local_sleeping = sleeping;
        if !sleeping {
            self.wake_requested = false;
            if self.chat_focused {
                self.close_chat();
            }
        }
    }

    /// Whether the bed screen owns input: the player lies in bed.
    pub(crate) const fn local_sleeping(&self) -> bool {
        self.local_sleeping
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
    fn sleeping_takes_ui_focus_and_wake_request_is_taken_once() {
        let mut runtime = UiRuntime::new(1);
        runtime.request_wake();
        assert!(!runtime.take_wake_request());

        runtime.set_local_sleeping(true);
        assert!(runtime.ui_focused() && !runtime.chat_focused());
        runtime.open_chat();
        runtime.request_wake();
        assert!(runtime.take_wake_request());
        assert!(!runtime.take_wake_request());

        runtime.request_wake();
        runtime.set_local_sleeping(false);
        assert!(!runtime.chat_focused());
        assert!(!runtime.take_wake_request());
    }
}
