//! Replays a Zeqa-like request-mode session (join, far teleport, same-area re-send) through the
//! real stream and reports presentation latency and transient-artifact frames.
//! Run with `cargo test -p client-world streaming_harness -- --ignored --nocapture`.

use std::collections::{HashMap, VecDeque};

use super::*;

const STONE: u32 = 1;
const AIR: u32 = 12_530;
const RADIUS: i32 = 8;
const FRAME: Duration = Duration::from_millis(8);
const REPLY_LATENCY_FRAMES: u64 = 3;
const MESH_JOBS_PER_FRAME: usize = 64;
const MAX_FRAMES: u64 = 4_000;
const NEAR_CHUNKS: f32 = 4.0;

/// Floating island one sub-chunk thick with pillars, void everywhere else.
fn solid(key: SubChunkKey) -> bool {
    key.y == 4 || (key.y == 5 && (key.x + key.z).rem_euclid(5) == 0)
}

fn sub_chunk_payload(y: i32) -> Vec<u8> {
    let mut payload = vec![9, 1, y as i8 as u8, 1];
    payload.extend(zig_zag_i32(STONE as i32));
    payload
}

fn spiral(center: ChunkKey, radius: i32) -> Vec<ChunkKey> {
    let mut columns = Vec::new();
    for dx in -radius..=radius {
        for dz in -radius..=radius {
            if dx * dx + dz * dz <= radius * radius {
                columns.push(ChunkKey::new(0, center.x + dx, center.z + dz));
            }
        }
    }
    columns.sort_by_key(|key| (key.x - center.x).pow(2) + (key.z - center.z).pow(2));
    columns
}

#[derive(Default, Debug, Clone, Copy)]
struct Report {
    frames_to_90: Option<u64>,
    frames_to_100: Option<u64>,
    millis_to_90: Option<u128>,
    millis_to_100: Option<u128>,
    /// Time until every in-view sub-chunk within `NEAR_CHUNKS` presents its converged mesh.
    millis_to_near: Option<u128>,
    poll_p95_us: u128,
    poll_p99_us: u128,
    poll_max_us: u128,
    artifact_frames: u64,
    dark_meshes: u64,
    geometry_meshes: u64,
    min_presented_in_view: usize,
    in_view: usize,
}

struct Published {
    key: SubChunkKey,
    frame: u64,
    mesh: Option<ChunkMesh>,
}

struct Harness {
    stream: WorldStream,
    sequence: u64,
    frame: u64,
    wire: VecDeque<WorldEvent>,
    replies: VecDeque<(u64, WorldEvent)>,
    presented: HashMap<SubChunkKey, ChunkMesh>,
    log: Vec<Published>,
    camera: [f32; 3],
    columns_per_frame: usize,
    frames_per_column: u64,
    frame_sleep: Duration,
    /// Columns whose sub-chunk replies are held until removed from this set.
    withheld: BTreeSet<ChunkKey>,
    held: Vec<(ChunkKey, WorldEvent)>,
    poll_times: Vec<Duration>,
}

impl Harness {
    /// The simulated server sends `columns_per_frame` columns on every `frames_per_column`-th frame.
    fn new(columns_per_frame: usize, frames_per_column: u64) -> Self {
        let stream = WorldStream::new(WorldBootstrap {
            dimension: 0,
            local_player_runtime_id: 1,
            local_player_unique_id: 1,
            player_position: [8.0, 81.62, 8.0],
            world_spawn_position: [8, 80, 8],
            air_network_id: AIR,
            block_network_ids_are_hashes: false,
        });
        Self {
            stream,
            sequence: 1,
            frame: 0,
            wire: VecDeque::new(),
            replies: VecDeque::new(),
            presented: HashMap::new(),
            log: Vec::new(),
            camera: [8.0, 81.62, 8.0],
            columns_per_frame,
            frames_per_column,
            frame_sleep: FRAME,
            withheld: BTreeSet::new(),
            held: Vec::new(),
            poll_times: Vec::new(),
        }
    }

    fn for_tests() -> Self {
        Self {
            frame_sleep: Duration::from_millis(1),
            ..Self::new(8, 1)
        }
    }

    fn push_column(&mut self, column: ChunkKey) {
        self.wire.push_back(WorldEvent::LevelChunk(LevelChunkEvent {
            dimension: 0,
            x: column.x,
            z: column.z,
            mode: LevelChunkMode::LimitedRequests { highest: 10 },
            payload: biome_payload(0, 1),
        }));
    }

    fn publications(&self, key: SubChunkKey) -> Vec<&ChunkMesh> {
        self.log
            .iter()
            .filter(|published| published.key == key)
            .filter_map(|published| published.mesh.as_ref())
            .collect()
    }

    fn step_until(&mut self, done: impl Fn(&Self) -> bool) {
        for _ in 0..MAX_FRAMES {
            if done(self) {
                return;
            }
            self.step();
        }
        panic!("condition not reached within {MAX_FRAMES} frames");
    }

    /// Queues the server's view announcement and every column around `center`, nearest first.
    fn send_view(&mut self, center: ChunkKey, teleport: bool) {
        let block = [center.x * 16 + 8, 80, center.z * 16 + 8];
        self.camera = [block[0] as f32, 81.62, block[2] as f32];
        if teleport {
            self.wire.push_back(WorldEvent::MovePlayer(MovePlayerEvent {
                runtime_id: 1,
                position: self.camera,
                pitch: 0.0,
                yaw: 0.0,
                head_yaw: 0.0,
                mode: MovePlayerMode::Teleport,
                on_ground: true,
                teleported: true,
                source_tick: 0,
            }));
        }
        self.wire.push_back(WorldEvent::ChunkRadiusUpdated(RADIUS));
        self.wire
            .push_back(WorldEvent::PublisherUpdate(PublisherUpdateEvent {
                center: block,
                radius_blocks: (RADIUS * 16) as u32,
            }));
        for column in spiral(center, RADIUS) {
            self.push_column(column);
        }
    }

    fn deliver(&mut self) {
        let (released, held) = std::mem::take(&mut self.held)
            .into_iter()
            .partition::<Vec<_>, _>(|(column, _)| !self.withheld.contains(column));
        self.held = held;
        for (_, event) in released {
            self.replies.push_back((self.frame, event));
        }
        while self
            .replies
            .front()
            .is_some_and(|(due, _)| *due <= self.frame)
        {
            let (_, event) = self.replies.pop_front().unwrap();
            self.wire.push_front(event);
        }
        let mut columns = 0;
        while let Some(event) = self.wire.front() {
            let is_column = matches!(event, WorldEvent::LevelChunk(_));
            if is_column
                && (columns == self.columns_per_frame
                    || !self.frame.is_multiple_of(self.frames_per_column))
            {
                break;
            }
            let creates_request = is_column;
            if self.stream.remaining_admission_capacity() == 0
                || (creates_request
                    && self.stream.pending_request_work_count() >= OUTBOUND_REQUEST_CAPACITY)
            {
                break;
            }
            let event = self.wire.pop_front().unwrap();
            self.stream
                .submit(self.sequence, event)
                .expect("admission was checked");
            self.sequence += 1;
            columns += usize::from(is_column);
        }
    }

    fn answer_requests(&mut self) {
        for request in self.stream.take_requests() {
            let sent_at = Instant::now();
            self.stream.record_sub_chunk_request_transport_pending(
                request.chunk,
                request.base_sub_chunk_y,
                request.count,
            );
            self.stream.acknowledge_sub_chunk_request_sent(
                request.chunk,
                request.base_sub_chunk_y,
                request.count,
                sent_at,
            );
            let entries = (0..request.count)
                .map(|offset| {
                    let y = request.base_sub_chunk_y + offset as i32;
                    let key = SubChunkKey::from_chunk(request.chunk, y);
                    SubChunkEntryEvent {
                        position: [key.x, y, key.z],
                        result: if solid(key) {
                            SubChunkResult::Success {
                                payload: sub_chunk_payload(y),
                            }
                        } else {
                            SubChunkResult::AllAir
                        },
                    }
                })
                .collect();
            let reply = WorldEvent::SubChunks(SubChunkBatchEvent {
                dimension: 0,
                entries,
            });
            if self.withheld.contains(&request.chunk) {
                self.held.push((request.chunk, reply));
            } else {
                self.replies
                    .push_back((self.frame + REPLY_LATENCY_FRAMES, reply));
            }
        }
    }

    fn present(&mut self) {
        while let Some(change) = self.stream.pop_mesh_change() {
            match change {
                WorldMeshChange::Upsert {
                    key,
                    mesh,
                    generation,
                    dirty_since,
                    ..
                } => {
                    self.stream.acknowledge_mesh_upload(
                        key,
                        generation,
                        dirty_since,
                        Instant::now(),
                    );
                    self.log.push(Published {
                        key,
                        frame: self.frame,
                        mesh: Some(mesh.clone()),
                    });
                    self.presented.insert(key, mesh);
                }
                WorldMeshChange::Remove {
                    key,
                    generation,
                    dirty_since,
                    ..
                } => {
                    self.stream.acknowledge_mesh_upload(
                        key,
                        generation,
                        dirty_since,
                        Instant::now(),
                    );
                    self.log.push(Published {
                        key,
                        frame: self.frame,
                        mesh: None,
                    });
                    self.presented.remove(&key);
                }
            }
        }
    }

    fn idle(&self) -> bool {
        self.wire.is_empty()
            && self.replies.is_empty()
            && self.held.is_empty()
            && self.stream.pending_decode.is_empty()
            && self.stream.in_flight_decode_jobs == 0
            && self.stream.pending_light.is_empty()
            && self.stream.in_flight_light.is_empty()
            && self.stream.pending_mesh.is_empty()
            && self.stream.in_flight.is_empty()
            && self.stream.mesh_changes.is_empty()
            && self.stream.staged_mesh_completions.is_empty()
            && self.stream.requested_sub_chunks.is_empty()
    }

    fn step(&mut self) {
        self.deliver();
        self.stream.set_view_forward([0.0, 0.0, 1.0]);
        let poll_started = Instant::now();
        let _ = self.stream.poll(self.camera, MESH_JOBS_PER_FRAME);
        self.poll_times.push(poll_started.elapsed());
        let _ = self.stream.take_committed_controls();
        self.answer_requests();
        self.present();
        if std::env::var_os("CINNABAR_HARNESS_TRACE").is_some() && self.frame.is_multiple_of(20) {
            println!(
                "frame {} wire {} decode {}/{} heavy {} requests {} light {}/{} mesh {}/{} presented {}",
                self.frame,
                self.wire.len(),
                self.stream.pending_decode.len(),
                self.stream.in_flight_decode_jobs,
                self.stream.heavy_sequences.len(),
                self.stream.requests.len(),
                self.stream.pending_light.len(),
                self.stream.in_flight_light.len(),
                self.stream.pending_mesh.len(),
                self.stream.in_flight.len(),
                self.presented.len(),
            );
        }
        self.frame += 1;
        std::thread::sleep(self.frame_sleep);
    }

    /// Runs until the stream is idle and reports against the converged meshes.
    fn run(&mut self) -> Report {
        let start_frame = self.frame;
        let started = Instant::now();
        let log_start = self.log.len();
        self.poll_times.clear();
        let initial = self.presented.clone();
        let mut frame_times = Vec::new();
        let mut presented_per_frame = Vec::new();
        let mut idle_frames = 0;
        while idle_frames < 8 && self.frame - start_frame < MAX_FRAMES {
            self.step();
            frame_times.push(started.elapsed());
            presented_per_frame.push(self.presented.keys().copied().collect::<BTreeSet<_>>());
            idle_frames = if self.idle() { idle_frames + 1 } else { 0 };
        }
        let final_meshes = self.presented.clone();
        let in_view = final_meshes
            .iter()
            .filter(|(key, mesh)| !mesh.cube_quads().is_empty() && in_frustum(**key, self.camera))
            .map(|(key, _)| *key)
            .collect::<BTreeSet<_>>();
        self.poll_times.sort_unstable();
        let mut report = Report {
            poll_p95_us: self.poll_times[self.poll_times.len() * 95 / 100].as_micros(),
            poll_p99_us: self.poll_times[self.poll_times.len() * 99 / 100].as_micros(),
            poll_max_us: self.poll_times.last().unwrap().as_micros(),
            in_view: in_view.len(),
            min_presented_in_view: usize::MAX,
            ..Report::default()
        };
        // Per in-view key: frame since which it has shown its final mesh, and windows in which
        // it showed a different one.
        let mut converged_at = HashMap::new();
        let mut shown_since = HashMap::<SubChunkKey, (u64, bool)>::new();
        let mut artifact_windows = Vec::new();
        for key in &in_view {
            if let Some(mesh) = initial.get(key) {
                let converged = Some(mesh) == final_meshes.get(key);
                shown_since.insert(*key, (start_frame, converged));
                if converged {
                    converged_at.insert(*key, start_frame);
                }
            }
        }
        for published in &self.log[log_start..] {
            if !in_view.contains(&published.key) {
                continue;
            }
            let final_mesh = &final_meshes[&published.key];
            if let Some((since, false)) = shown_since.remove(&published.key) {
                artifact_windows.push((since, published.frame));
            }
            let Some(mesh) = &published.mesh else {
                converged_at.remove(&published.key);
                continue;
            };
            let converged = mesh == final_mesh;
            if !converged {
                if std::env::var_os("CINNABAR_HARNESS_TRACE").is_some() {
                    println!(
                        "artifact {:?} frame {} zero {}->{} quads {}->{}",
                        published.key,
                        published.frame,
                        zero_light_samples(mesh),
                        zero_light_samples(final_mesh),
                        mesh.cube_quads().len(),
                        final_mesh.cube_quads().len(),
                    );
                }
                if zero_light_samples(mesh) > zero_light_samples(final_mesh) {
                    report.dark_meshes += 1;
                }
                if mesh.cube_quads() != final_mesh.cube_quads() {
                    report.geometry_meshes += 1;
                }
            }
            shown_since.insert(published.key, (published.frame, converged));
            if converged {
                converged_at.entry(published.key).or_insert(published.frame);
            } else {
                converged_at.remove(&published.key);
            }
        }
        let camera = self.camera;
        let near = in_view
            .iter()
            .filter(|key| {
                (key.x as f32 * 16.0 + 8.0 - camera[0]).hypot(key.z as f32 * 16.0 + 8.0 - camera[2])
                    <= NEAR_CHUNKS * 16.0
            })
            .collect::<Vec<_>>();
        for offset in 0..presented_per_frame.len() {
            let frame = start_frame + offset as u64;
            let converged_by = |key: &&SubChunkKey| {
                converged_at
                    .get(*key)
                    .is_some_and(|converged| *converged <= frame)
            };
            if artifact_windows
                .iter()
                .any(|(from, to)| (*from..*to).contains(&frame))
            {
                report.artifact_frames += 1;
            }
            let presented = in_view.iter().filter(converged_by).count();
            let shown = presented_per_frame[offset]
                .iter()
                .filter(|key| in_view.contains(key))
                .count();
            report.min_presented_in_view = report.min_presented_in_view.min(shown);
            let elapsed = frame_times[offset].as_millis();
            if report.frames_to_90.is_none() && presented * 10 >= in_view.len() * 9 {
                report.frames_to_90 = Some(offset as u64 + 1);
                report.millis_to_90 = Some(elapsed);
            }
            if report.millis_to_near.is_none() && near.iter().all(converged_by) {
                report.millis_to_near = Some(elapsed);
            }
            if report.frames_to_100.is_none() && presented == in_view.len() {
                report.frames_to_100 = Some(offset as u64 + 1);
                report.millis_to_100 = Some(elapsed);
            }
        }
        report
    }
}

fn in_frustum(key: SubChunkKey, camera: [f32; 3]) -> bool {
    let center = [
        key.x as f32 * 16.0 + 8.0 - camera[0],
        key.z as f32 * 16.0 + 8.0 - camera[2],
    ];
    let distance = center[0].hypot(center[1]);
    // Yaw 0 faces +Z; 90° horizontal field of view, padded by one sub-chunk radius.
    distance <= (RADIUS * 16) as f32 && (distance < 12.0 || center[1] >= center[0].abs() - 12.0)
}

fn zero_light_samples(mesh: &ChunkMesh) -> usize {
    mesh.cube_lighting()
        .iter()
        .flat_map(|lighting| lighting.samples())
        .filter(|sample| sample & 0xff == 0)
        .count()
}

#[test]
#[ignore = "timing harness; run explicitly with --ignored --nocapture"]
fn streaming_harness_reports_teleport_and_resend() {
    println!("rayon threads: {}", rayon::current_num_threads());
    for (columns_per_frame, frames_per_column) in [(8, 1), (1, 4)] {
        let mut harness = Harness::new(columns_per_frame, frames_per_column);
        let columns_per_frame = format!("{columns_per_frame}/{frames_per_column}");
        harness.send_view(ChunkKey::new(0, 0, 0), false);
        let join = harness.run();
        println!("[{columns_per_frame} col per frames] join: {join:?}");
        assert_eq!((join.dark_meshes, join.geometry_meshes), (0, 0));

        harness.send_view(ChunkKey::new(0, 125, 137), true);
        let teleport = harness.run();
        println!("[{columns_per_frame} col per frames] far teleport: {teleport:?}");
        assert_eq!((teleport.dark_meshes, teleport.geometry_meshes), (0, 0));

        harness.send_view(ChunkKey::new(0, 127, 137), true);
        let resend = harness.run();
        println!("[{columns_per_frame} col per frames] near teleport with re-send: {resend:?}");
        assert_eq!((resend.dark_meshes, resend.geometry_meshes), (0, 0));
        assert!(
            resend.min_presented_in_view * 10 >= resend.in_view * 8,
            "a re-send of the same area must not drop presented meshes"
        );
    }
}

fn island(chunk: ChunkKey) -> SubChunkKey {
    SubChunkKey::from_chunk(chunk, 4)
}

/// Steps past the quiet-stream grace so unannounced neighbours stop being due.
fn step_past_quiet_grace(harness: &mut Harness) {
    let quiet_since = Instant::now();
    harness.step_until(|_| quiet_since.elapsed() > UNSENT_COLUMN_GRACE + FRAME);
}

#[test]
fn mesh_waits_for_a_requested_neighbour_and_publishes_once() {
    let (a, b) = (ChunkKey::new(0, 0, 0), ChunkKey::new(0, 1, 0));
    let mut harness = Harness::for_tests();
    harness.withheld.insert(b);
    harness.push_column(a);
    harness.push_column(b);
    harness.step_until(|harness| harness.stream.light_is_current(island(a)));
    step_past_quiet_grace(&mut harness);
    assert!(harness.publications(island(a)).is_empty());

    harness.withheld.clear();
    harness.step_until(|harness| harness.idle());
    assert_eq!(harness.publications(island(a)).len(), 1);
}

#[test]
fn late_neighbour_rebuilds_the_published_mesh() {
    let (a, b) = (ChunkKey::new(0, 0, 0), ChunkKey::new(0, 1, 0));
    let mut harness = Harness::for_tests();
    harness.push_column(a);
    harness.step_until(|harness| !harness.publications(island(a)).is_empty());
    let walled = harness.publications(island(a))[0].cube_quads().len();

    harness.push_column(b);
    harness.step_until(|harness| harness.publications(island(a)).len() == 2);
    assert!(harness.publications(island(a))[1].cube_quads().len() < walled);
}

#[test]
fn absent_neighbour_light_reads_the_dimension_default() {
    let key = SubChunkKey::new(0, 0, 4, 0);
    let overworld = super::MeshLightHalo {
        center: Some(key),
        ..Default::default()
    };
    assert_eq!(overworld.sample_channels([16, 0, 0]), [0, 15]);
    let nether = super::MeshLightHalo {
        center: Some(SubChunkKey::new(1, 0, 4, 0)),
        ..Default::default()
    };
    assert_eq!(nether.sample_channels([16, 0, 0]), [0, 0]);
}

#[test]
fn disjoint_teleport_keeps_columns_the_destination_view_covers() {
    let mut stream = WorldStream::new(WorldBootstrap {
        dimension: 0,
        local_player_runtime_id: 1,
        local_player_unique_id: 1,
        player_position: [0.5, 70.0, 0.5],
        world_spawn_position: [0, 70, 0],
        air_network_id: AIR,
        block_network_ids_are_hashes: false,
    });
    stream.chunk_radius = Some(8);
    stream.publisher_radius_chunks = Some(8);
    let kept = SubChunkKey::new(0, 10, -4, 0);
    let dropped = SubChunkKey::new(0, -8, -4, 0);
    for key in [kept, dropped] {
        stream.loaded_columns.insert(key.chunk());
        stream.resident.insert(key);
    }

    stream
        .submit(
            1,
            WorldEvent::MovePlayer(MovePlayerEvent {
                runtime_id: 1,
                position: [224.5, 70.0, 0.5],
                mode: MovePlayerMode::Teleport,
                teleported: true,
                ..Default::default()
            }),
        )
        .unwrap();

    assert!(stream.provisional_publisher_rebase);
    assert!(stream.tracked_columns().contains(&kept.chunk()));
    assert!(!stream.tracked_columns().contains(&dropped.chunk()));
}

#[test]
fn scheduler_serves_sub_chunks_in_view_before_nearer_ones_behind() {
    let view = super::SchedulerView {
        position: [8.0, 72.0, 8.0],
        forward: Some([0.0, 0.0, 1.0]),
    };
    let ahead = SubChunkKey::new(0, 0, 4, 3);
    let behind = SubChunkKey::new(0, 0, 4, -2);
    let mut candidates = BinaryHeap::from([
        PendingSchedulerCandidate::new(behind, 1, view, false),
        PendingSchedulerCandidate::new(ahead, 1, view, false),
    ]);

    assert_eq!(candidates.pop().map(|candidate| candidate.key), Some(ahead));
}

/// Measures the destination's visible ring, including admission and mesh convergence.
#[test]
#[ignore = "release teleport timing; run with --ignored --nocapture"]
fn teleport_visible_ring_timing() {
    let mut harness = Harness::new(8, 1);
    harness.send_view(ChunkKey::new(0, 0, 0), false);
    harness.run();
    harness.send_view(ChunkKey::new(0, 125, 137), true);
    let report = harness.run();
    println!("teleport_visible_ring {report:?}");
    assert!(report.in_view > 0);
    assert!(report.millis_to_near.is_some());
    assert!(report.millis_to_100.is_some());
    assert_eq!(report.dark_meshes, 0);
    assert_eq!(report.geometry_meshes, 0);
    assert!(report.poll_p95_us <= report.poll_p99_us);
    assert!(report.poll_p99_us <= report.poll_max_us);
}

/// Measures logical removal of a full retained disk without network or GPU work.
#[test]
#[ignore = "release eviction timing; run with --ignored --nocapture"]
fn teleport_eviction_timing() {
    let mut times = Vec::new();
    for _ in 0..5 {
        let mut harness = Harness::new(8, 1);
        for column in spiral(ChunkKey::new(0, 0, 0), 16) {
            harness.stream.loaded_columns.insert(column);
            for y in -4..20 {
                harness
                    .stream
                    .record_known_air(SubChunkKey::from_chunk(column, y));
            }
        }
        let before = Instant::now();
        harness.stream.evict_all_resident();
        times.push(before.elapsed().as_micros());
        assert!(harness.stream.resident.is_empty());
        assert!(harness.stream.connectivity.is_empty());
    }
    println!("teleport_eviction_us {times:?}");
}

/// Batch retirement preserves overlap and snapshots while invalidating removed authority.
#[test]
fn batch_eviction_preserves_overlap_and_snapshot() {
    let mut harness = Harness::for_tests();
    let removed = SubChunkKey::new(0, 0, 4, 0);
    let retained = SubChunkKey::new(0, 1, 4, 0);
    for key in [removed, retained] {
        harness
            .stream
            .store
            .commit_sub_chunk(key, uniform_sub_chunk(STONE))
            .unwrap();
        harness.stream.sync_resident(key);
    }
    let snapshot = harness.stream.store.sub_chunk(removed).unwrap();
    let revision = harness.stream.store.collision_revision(retained.chunk());
    harness
        .stream
        .evict_columns(BTreeSet::from([removed.chunk()]));
    assert!(harness.stream.store.sub_chunk(removed).is_none());
    assert!(
        harness
            .stream
            .store
            .collision_revision(removed.chunk())
            .is_none()
    );
    assert_eq!(
        harness.stream.store.collision_revision(retained.chunk()),
        revision
    );
    assert!(harness.stream.resident.contains(&retained));
    assert!(harness.stream.pending_mesh.contains_key(&removed));
    assert!(harness.stream.pending_mesh.contains_key(&retained));
    assert_eq!(snapshot.runtime_id(0, 0, 0, 0), Some(STONE));
}
