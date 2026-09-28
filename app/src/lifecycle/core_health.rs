//! Core-process supervision helpers: crash-loop backoff and a size-bounded stderr log.

use std::{
    fs::{self, File, OpenOptions},
    time::Duration,
};

use crate::install_layout::InstallLayout;

const BASE_DELAY: Duration = Duration::from_millis(500);
const MAX_DELAY: Duration = Duration::from_secs(8);
const MAX_CONSECUTIVE_FAILURES: u32 = 5;
const STABLE_RUN: Duration = Duration::from_secs(60);
const MAX_LOG_BYTES: u64 = 1 << 20;

/// Exponential restart delay that resets once a core stays up for a stable interval.
#[derive(Debug, Default)]
pub(crate) struct RestartBackoff {
    failures: u32,
}

impl RestartBackoff {
    /// Delay before the next restart after a core that ran for `ran_for` died; `None` means stop retrying.
    // Consumed by the session reconnect path once the core is restarted mid-session.
    #[allow(dead_code)]
    pub(crate) fn next_delay(&mut self, ran_for: Duration) -> Option<Duration> {
        if ran_for >= STABLE_RUN {
            self.failures = 0;
        }
        self.failures += 1;
        if self.failures > MAX_CONSECUTIVE_FAILURES {
            return None;
        }
        Some((BASE_DELAY * 2u32.pow(self.failures - 1)).min(MAX_DELAY))
    }
}

/// Opens the core's append-only stderr log, rotating one previous generation past the size bound.
pub(crate) fn open_core_log(layout: &InstallLayout) -> Option<File> {
    let dir = layout.log_dir();
    fs::create_dir_all(&dir).ok()?;
    let path = dir.join("core.log");
    if fs::metadata(&path).is_ok_and(|meta| meta.len() > MAX_LOG_BYTES) {
        let _ = fs::rename(&path, dir.join("core.log.1"));
    }
    OpenOptions::new().create(true).append(true).open(path).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn delay_doubles_caps_and_gives_up() {
        let mut backoff = RestartBackoff::default();
        let quick = Duration::from_secs(1);
        assert_eq!(backoff.next_delay(quick), Some(Duration::from_millis(500)));
        assert_eq!(backoff.next_delay(quick), Some(Duration::from_secs(1)));
        assert_eq!(backoff.next_delay(quick), Some(Duration::from_secs(2)));
        assert_eq!(backoff.next_delay(quick), Some(Duration::from_secs(4)));
        assert_eq!(backoff.next_delay(quick), Some(Duration::from_secs(8)));
        assert_eq!(backoff.next_delay(quick), None);
    }

    #[test]
    fn a_stable_run_resets_the_failure_count() {
        let mut backoff = RestartBackoff::default();
        for _ in 0..MAX_CONSECUTIVE_FAILURES {
            backoff.next_delay(Duration::from_secs(1));
        }
        assert_eq!(backoff.next_delay(STABLE_RUN), Some(BASE_DELAY));
    }
}
