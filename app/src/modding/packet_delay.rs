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
    use mod_host::ModGrants;

    /// A component whose `init` requests `delay` milliseconds of packet delay.
    fn delaying(directory: &std::path::Path, index: usize, delay: u32) -> PathBuf {
        let package = include_str!("../../../crates/mod-api/wit/extension.wit")
            .lines()
            .next()
            .unwrap()
            .trim_start_matches("package ")
            .trim_end_matches(';');
        let (name, version) = package.split_once('@').unwrap();
        let source = format!(
            r#"(component
  (import "{name}/gameplay@{version}" (instance $gameplay
    (export "set-packet-delay" (func (param "delay-ms" u32) (result (result (error string)))))))
  (alias export $gameplay "set-packet-delay" (func $packet-delay))
  (core module $memory-module
    (memory (export "memory") 1)
    (global $next (mut i32) (i32.const 4096))
    (func (export "realloc") (param i32 i32 i32 i32) (result i32)
      (local $old i32)
      global.get $next local.tee $old
      local.get 3 i32.add global.set $next local.get $old))
  (core instance $mem (instantiate $memory-module))
  (alias core export $mem "memory" (core memory $memory))
  (alias core export $mem "realloc" (core func $realloc))
  (core func $lower-delay (canon lower (func $packet-delay) (memory $memory) (realloc $realloc)))
  (core module $code
    (import "host" "delay" (func $delay (param i32 i32)))
    (func (export "init") i32.const {delay} i32.const 256 call $delay)
    (func (export "frame")))
  (core instance $host (export "delay" (func $lower-delay)))
  (core instance $run (instantiate $code (with "host" (instance $host))))
  (func (export "init") (canon lift (core func $run "init")))
  (func (export "frame") (canon lift (core func $run "frame"))))"#
        );
        let path = directory.join(format!("delay-{index}.wat"));
        std::fs::write(&path, source).unwrap();
        path
    }

    #[test]
    fn the_earliest_granted_non_zero_packet_delay_wins_across_loaded_mods() {
        let directory = tempfile::tempdir().unwrap();
        let granted = ModGrants {
            packet_delay: true,
            ..Default::default()
        };
        let mods = [(0, true), (300, false), (200, true), (500, true)]
            .into_iter()
            .enumerate()
            .map(|(index, (delay, grant))| {
                let grants = if grant {
                    granted.clone()
                } else {
                    ModGrants::default()
                };
                (delaying(directory.path(), index, delay), grants)
            })
            .collect();
        let mut app = App::new();
        super::super::configure_set(&mut app, mods);
        let mut runtime = app.world_mut().resource_mut::<ModRuntime>();
        assert_eq!(runtime.host_count(), 4);
        assert_eq!(requested(Some(&runtime)), 200);
        runtime.suspended = true;
        assert_eq!(requested(Some(&runtime)), 0);
    }

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
