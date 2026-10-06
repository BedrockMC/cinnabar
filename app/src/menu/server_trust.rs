//! The core's server trust question: sending the player's answer off the frame thread, and polling
//! a per-session core, which no account link watches, for its question while it prepares a join.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use crossbeam_channel::{RecvTimeoutError, Sender, bounded};
use launcher::menu::view::ServerTrustPrompt;
use protocol::launcher_control::{answer_server_trust, poll_events};

/// How often a per-session core's question is polled, matching the join's event polling.
const POLL_INTERVAL: Duration = Duration::from_millis(250);

/// Answers trust prompt `id`; a lost answer leaves the prompt to end with its join.
pub(crate) fn send(socket_dir: PathBuf, id: u64, trusted: bool) {
    thread::spawn(move || {
        let Some(runtime) = runtime() else {
            return;
        };
        if let Err(error) = runtime.block_on(answer_server_trust(&socket_dir, id, trusted)) {
            bevy::log::warn!(%error, "server trust answer was not delivered");
        }
    });
}

fn runtime() -> Option<tokio::runtime::Runtime> {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .ok()
}

/// Where the menu reads a pending trust question and sends its answer.
pub(crate) trait TrustSource {
    fn prompt(&self) -> Option<ServerTrustPrompt>;
    fn answer(&self, id: u64, trusted: bool);
}

#[derive(Default)]
struct Watched {
    prompt: Option<ServerTrustPrompt>,
    answered: Option<u64>,
}

/// Polls one per-session core's events for its trust question until dropped.
pub(crate) struct SessionTrust {
    socket_dir: PathBuf,
    watched: Arc<Mutex<Watched>>,
    _stop: Sender<()>,
}

impl std::fmt::Debug for SessionTrust {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("SessionTrust")
            .field("socket_dir", &self.socket_dir)
            .finish_non_exhaustive()
    }
}

impl SessionTrust {
    pub(crate) fn watch(socket_dir: PathBuf) -> Self {
        let watched = Arc::new(Mutex::new(Watched::default()));
        let (stop, stopped) = bounded::<()>(0);
        let (shared, dir) = (Arc::clone(&watched), socket_dir.clone());
        thread::spawn(move || {
            let Some(runtime) = runtime() else {
                return;
            };
            while let Err(RecvTimeoutError::Timeout) = stopped.recv_timeout(POLL_INTERVAL) {
                let Ok(events) = runtime.block_on(poll_events(&dir)) else {
                    continue;
                };
                let prompt = events.server_trust.map(|prompt| ServerTrustPrompt {
                    id: prompt.id,
                    url: prompt.url,
                    from_session_core: true,
                });
                shared
                    .lock()
                    .unwrap_or_else(|poison| poison.into_inner())
                    .prompt = prompt;
            }
        });
        Self {
            socket_dir,
            watched,
            _stop: stop,
        }
    }

    fn with<T>(&self, read: impl FnOnce(&mut Watched) -> T) -> T {
        read(
            &mut self
                .watched
                .lock()
                .unwrap_or_else(|poison| poison.into_inner()),
        )
    }
}

impl TrustSource for SessionTrust {
    fn prompt(&self) -> Option<ServerTrustPrompt> {
        self.with(|watched| {
            let prompt = watched.prompt.as_ref()?;
            (watched.answered != Some(prompt.id)).then(|| prompt.clone())
        })
    }

    fn answer(&self, id: u64, trusted: bool) {
        self.with(|watched| watched.answered = Some(id));
        send(self.socket_dir.clone(), id, trusted);
    }
}
