//! Coalesced private core control, off the frame thread and independent of UI focus.

use super::ModRuntime;
use crate::runtime::network::NetworkHandle;
use bevy::prelude::*;
use crossbeam_channel::{Sender, bounded};
use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU32, Ordering},
    },
    time::Duration,
};

const HEARTBEAT: Duration = Duration::from_secs(1);
const REQUEST_TIMEOUT: Duration = Duration::from_millis(500);

struct Shared {
    delay: AtomicU32,
    stop: AtomicBool,
}

pub(super) struct Worker {
    endpoint: PathBuf,
    shared: Arc<Shared>,
    wake: Sender<()>,
}

impl Drop for Worker {
    fn drop(&mut self) {
        self.shared.delay.store(0, Ordering::Release);
        self.shared.stop.store(true, Ordering::Release);
        let _ = self.wake.try_send(());
    }
}

impl Worker {
    fn start(endpoint: PathBuf) -> Option<Self> {
        let shared = Arc::new(Shared {
            delay: AtomicU32::new(0),
            stop: AtomicBool::new(false),
        });
        let (wake, receiver) = bounded(1);
        let state = Arc::clone(&shared);
        let socket_dir = endpoint.clone();
        std::thread::Builder::new()
            .name("mod-packet-delay".into())
            .spawn(move || {
                let Ok(runtime) = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                else {
                    return;
                };
                let mut last = None;
                loop {
                    let stop = state.stop.load(Ordering::Acquire);
                    let delay = if stop {
                        0
                    } else {
                        state.delay.load(Ordering::Acquire)
                    };
                    if stop || last != Some(delay) || delay != 0 {
                        let success = runtime
                            .block_on(async {
                                tokio::time::timeout(
                                    REQUEST_TIMEOUT,
                                    protocol::launcher_control::set_packet_delay(
                                        &socket_dir,
                                        delay,
                                    ),
                                )
                                .await
                            })
                            .is_ok_and(|result| {
                                result.is_ok_and(|lease| {
                                    lease.delay_ms == delay && lease.lease_ms > 0
                                })
                            });
                        last = success.then_some(delay);
                    }
                    if stop {
                        return;
                    }
                    let _ = receiver.recv_timeout(HEARTBEAT);
                }
            })
            .ok()?;
        Some(Self {
            endpoint,
            shared,
            wake,
        })
    }

    fn publish(&self, delay: u32) {
        if self.shared.delay.swap(delay, Ordering::AcqRel) != delay {
            let _ = self.wake.try_send(());
        }
    }
}

/// The earliest active, granted mod with a non-zero delay wins, as for other single-valued output.
fn requested(extension: Option<&ModRuntime>) -> u32 {
    let Some(runtime) = extension.filter(|runtime| !runtime.suspended) else {
        return 0;
    };
    (0..runtime.host_count())
        .map(|index| runtime.host(index))
        .filter(|host| host.is_active() && host.grants().packet_delay)
        .map(mod_host::ModHost::packet_delay_ms)
        .find(|&delay| delay != 0)
        .unwrap_or(0)
}

pub(super) fn publish_packet_delay(
    extension: Option<Res<ModRuntime>>,
    network: Option<Res<NetworkHandle>>,
    mut worker: Local<Option<Worker>>,
) {
    let endpoint = network.as_deref().and_then(NetworkHandle::core_socket_dir);
    if worker
        .as_ref()
        .is_some_and(|worker| Some(worker.endpoint.as_path()) != endpoint)
    {
        *worker = None;
    }
    let delay = requested(extension.as_deref());
    if worker.is_none()
        && delay != 0
        && let Some(endpoint) = endpoint
    {
        *worker = Worker::start(endpoint.to_owned());
    }
    if let Some(worker) = worker.as_ref() {
        worker.publish(delay);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn coalesced_disable_cannot_be_lost_behind_a_full_wake_queue() {
        let (wake, receiver) = bounded(1);
        let worker = Worker {
            endpoint: "fixture".into(),
            shared: Arc::new(Shared {
                delay: AtomicU32::new(0),
                stop: AtomicBool::new(false),
            }),
            wake,
        };
        worker.publish(200);
        worker.publish(400);
        worker.publish(0);
        assert_eq!(receiver.len(), 1);
        assert_eq!(worker.shared.delay.load(Ordering::Acquire), 0);
        let shared = Arc::clone(&worker.shared);
        drop(worker);
        assert!(shared.stop.load(Ordering::Acquire));
        assert_eq!(shared.delay.load(Ordering::Acquire), 0);
    }
}
