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
        let _ = self.stream.poll(self.camera, MESH_JOBS_PER_FRAME);
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
        let mut report = Report {
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
        for offset in 0..presented_per_frame.len() {
            let frame = start_frame + offset as u64;
            if artifact_windows
                .iter()
                .any(|(from, to)| (*from..*to).contains(&frame))
            {
                report.artifact_frames += 1;
            }
            let presented = in_view
                .iter()
                .filter(|key| {
                    converged_at
                        .get(key)
                        .is_some_and(|converged| *converged <= frame)
                })
                .count();
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

        harness.send_view(ChunkKey::new(0, 125, 137), true);
        let teleport = harness.run();
        println!("[{columns_per_frame} col per frames] far teleport: {teleport:?}");

        harness.send_view(ChunkKey::new(0, 127, 137), true);
        let resend = harness.run();
        println!("[{columns_per_frame} col per frames] near teleport with re-send: {resend:?}");
        assert!(
            resend.min_presented_in_view * 10 >= resend.in_view * 8,
            "a re-send of the same area must not drop presented meshes"
        );
    }
}
