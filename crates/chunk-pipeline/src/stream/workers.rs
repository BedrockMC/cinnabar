mod priority;

use std::collections::VecDeque;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::{Arc, Condvar, LazyLock, Mutex, MutexGuard};
use std::time::{Duration, Instant};

/// Cores left to the frame (main and render threads).
const FRAME_CORES: usize = 2;
const MIN_WORLD_THREADS: usize = 3;
/// A lower lane's oldest job jumps the priority order after waiting this long, so
/// sustained mesh load cannot starve the decode and light work that mesh depends on.
const DECODE_MAX_WAIT: Duration = Duration::from_millis(4);
const LIGHT_MAX_WAIT: Duration = Duration::from_millis(16);

/// Work classes in scheduling order: mesh gates chunks appearing, decode feeds it, light trails.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Lane {
    Mesh,
    Decode,
    Light,
}

/// Thread split for one machine; only background threads take light, so they cap its width.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct PoolSize {
    pub(super) foreground: usize,
    pub(super) background: usize,
}

impl PoolSize {
    pub(super) fn for_cores(cores: usize) -> Self {
        let threads = cores.saturating_sub(FRAME_CORES).max(MIN_WORLD_THREADS);
        let background = (threads * 2 / 3).max(super::MIN_EFFECTIVE_LIGHT_JOB_CAP);
        Self {
            foreground: threads - background,
            background,
        }
    }

    pub(super) const fn threads(self) -> usize {
        self.foreground + self.background
    }
}

type Job = Box<dyn FnOnce() + Send + 'static>;
type Queue = VecDeque<(Instant, Job)>;

#[derive(Default)]
struct Queues {
    mesh: Queue,
    decode: Queue,
    light: Queue,
    shutdown: bool,
}

impl Queues {
    fn push(&mut self, lane: Lane, queued_at: Instant, job: Job) {
        let queue = match lane {
            Lane::Mesh => &mut self.mesh,
            Lane::Decode => &mut self.decode,
            Lane::Light => &mut self.light,
        };
        queue.push_back((queued_at, job));
    }

    /// Next job for a worker at time `now`; the clock is a parameter so ageing is testable.
    fn take(&mut self, background: bool, now: Instant) -> Option<Job> {
        let overdue = |queue: &Queue, limit| {
            queue
                .front()
                .is_some_and(|(queued, _)| now.saturating_duration_since(*queued) >= limit)
        };
        let lane = if background && overdue(&self.light, LIGHT_MAX_WAIT) {
            &mut self.light
        } else if overdue(&self.decode, DECODE_MAX_WAIT) || self.mesh.is_empty() {
            if self.decode.is_empty() && background {
                &mut self.light
            } else {
                &mut self.decode
            }
        } else {
            &mut self.mesh
        };
        lane.pop_front().map(|(_, job)| job)
    }
}

#[derive(Default)]
struct Shared {
    queues: Mutex<Queues>,
    ready: Condvar,
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, Queues> {
        self.queues
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

/// One world pool: threads prefer mesh, then decode, then light, unless a lower lane is
/// overdue. Light runs only on the lowered-priority background threads.
pub(super) struct WorldPool {
    shared: Arc<Shared>,
    size: PoolSize,
}

pub(super) static WORKERS: LazyLock<WorldPool> = LazyLock::new(|| {
    let cores = std::thread::available_parallelism().map_or(1, usize::from);
    WorldPool::new(PoolSize::for_cores(cores))
});

impl WorldPool {
    fn new(size: PoolSize) -> Self {
        let shared = Arc::new(Shared::default());
        for (name, count, background) in [
            ("world", size.foreground, false),
            ("world-bg", size.background, true),
        ] {
            for index in 0..count {
                let shared = Arc::clone(&shared);
                std::thread::Builder::new()
                    .name(format!("{name}-{index}"))
                    .spawn(move || work(&shared, name, background))
                    .expect("world worker could not start");
            }
        }
        Self { shared, size }
    }

    pub(super) const fn size(&self) -> PoolSize {
        self.size
    }

    pub(super) fn spawn(&self, lane: Lane, job: impl FnOnce() + Send + 'static) {
        self.shared.lock().push(lane, Instant::now(), Box::new(job));
        // A foreground waiter cannot take light, so light wakes everyone.
        if lane == Lane::Light {
            self.shared.ready.notify_all();
        } else {
            self.shared.ready.notify_one();
        }
    }
}

impl Drop for WorldPool {
    fn drop(&mut self) {
        self.shared.lock().shutdown = true;
        self.shared.ready.notify_all();
    }
}

fn work(shared: &Shared, name: &str, background: bool) {
    if background && let Err(error) = priority::lower() {
        eprintln!("{name}: could not lower worker priority: {error}");
    }
    let mut queues = shared.lock();
    loop {
        if queues.shutdown {
            return;
        }
        let Some(job) = queues.take(background, Instant::now()) else {
            queues = shared
                .ready
                .wait(queues)
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            continue;
        };
        drop(queues);
        // Matches rayon's default: a panicking world job aborts rather than losing its permits.
        if catch_unwind(AssertUnwindSafe(job)).is_err() {
            std::process::abort();
        }
        queues = shared.lock();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Mesh can use every world thread while light stays on the background share.
    #[test]
    fn pool_sizing_leaves_frame_cores_and_foreground_threads() {
        let sizes = [4, 8, 12].map(PoolSize::for_cores);
        assert_eq!(
            sizes[0],
            PoolSize {
                foreground: 1,
                background: 2
            }
        );
        assert_eq!(
            sizes[1],
            PoolSize {
                foreground: 2,
                background: 4
            }
        );
        assert_eq!(
            sizes[2],
            PoolSize {
                foreground: 4,
                background: 6
            }
        );
        for size in [1, 2, 3, 64].map(PoolSize::for_cores) {
            assert!(size.foreground >= 1);
            assert!(size.background >= super::super::MIN_EFFECTIVE_LIGHT_JOB_CAP);
        }
    }

    /// Saturating every light worker leaves both latency-sensitive lanes available.
    #[test]
    fn saturated_lighting_cannot_queue_ahead_of_mesh_or_decode() {
        let pool = WorldPool::new(PoolSize::for_cores(8));
        let (started_tx, started_rx) = crossbeam_channel::unbounded();
        let (release_tx, release_rx) = crossbeam_channel::unbounded();
        // More light jobs than threads: none may run on a foreground thread.
        for _ in 0..pool.size().threads() {
            let started = started_tx.clone();
            let release = release_rx.clone();
            // Surplus jobs may start after the test returns, so they must not panic.
            pool.spawn(Lane::Light, move || {
                let _ = started.send(());
                let _ = release.recv_timeout(Duration::from_secs(5));
            });
        }
        for _ in 0..pool.size().background {
            started_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        }
        assert!(started_rx.recv_timeout(Duration::from_millis(50)).is_err());
        let (done_tx, done_rx) = crossbeam_channel::unbounded();
        let mesh_done = done_tx.clone();
        pool.spawn(Lane::Mesh, move || mesh_done.send(()).unwrap());
        pool.spawn(Lane::Decode, move || done_tx.send(()).unwrap());
        let completed = (0..2).all(|_| done_rx.recv_timeout(Duration::from_secs(2)).is_ok());
        for _ in 0..pool.size().threads() {
            release_tx.send(()).unwrap();
        }
        assert!(completed, "lighting blocked another worker lane");
    }

    /// Pushes `queued` lanes at `queued_at` and drains them at `now` in worker order.
    fn take_order(
        background: bool,
        queued: &[Lane],
        queued_at: Instant,
        now: Instant,
    ) -> Vec<Lane> {
        let mut queues = Queues::default();
        let (order_tx, order_rx) = crossbeam_channel::unbounded();
        for &lane in queued {
            let order = order_tx.clone();
            queues.push(lane, queued_at, Box::new(move || order.send(lane).unwrap()));
        }
        while let Some(job) = queues.take(background, now) {
            job();
        }
        order_rx.try_iter().collect()
    }

    /// Fresh work runs mesh, then decode, then light; foreground workers never take light.
    #[test]
    fn fresh_work_follows_lane_priority() {
        let queued = [Lane::Light, Lane::Decode, Lane::Mesh, Lane::Mesh];
        let at = Instant::now();
        assert_eq!(
            take_order(true, &queued, at, at),
            [Lane::Mesh, Lane::Mesh, Lane::Decode, Lane::Light]
        );
        assert_eq!(
            take_order(false, &queued, at, at),
            [Lane::Mesh, Lane::Mesh, Lane::Decode]
        );
    }

    /// Overdue light and decode jump a mesh flood; overdue light still never runs foreground.
    #[test]
    fn overdue_lower_lanes_jump_a_mesh_flood() {
        let queued = [Lane::Light, Lane::Decode, Lane::Mesh, Lane::Mesh];
        let at = Instant::now();
        let decode_late = at + DECODE_MAX_WAIT;
        assert_eq!(
            take_order(true, &queued, at, decode_late),
            [Lane::Decode, Lane::Mesh, Lane::Mesh, Lane::Light]
        );
        let light_late = at + LIGHT_MAX_WAIT;
        assert_eq!(
            take_order(true, &queued, at, light_late),
            [Lane::Light, Lane::Decode, Lane::Mesh, Lane::Mesh]
        );
        assert_eq!(
            take_order(false, &queued, at, light_late),
            [Lane::Decode, Lane::Mesh, Lane::Mesh]
        );
    }
}
