use super::*;

/// Times a mesh-heavy transfer after lighting converges, including frame-paced acceptance.
#[test]
fn ready_mesh_backlog_drains() {
    let mut stream = lit_stream(1);
    let count = 2_048;
    for index in 0..count {
        let key = SubChunkKey::new(1, (index % 64) * 4, 0, (index / 64) * 4);
        stream
            .store
            .commit_sub_chunk(key, super::uniform_sub_chunk(2))
            .unwrap();
        install_current_light(&mut stream, key, 0, 0, false);
        stream.mark_dirty_exact(key, Instant::now());
    }
    let started = Instant::now();
    while !stream.pending_mesh.is_empty() || !stream.in_flight.is_empty() {
        stream.poll([0.0; 3], 64);
        acknowledge_mesh_changes(&mut stream);
        assert!(
            started.elapsed() < Duration::from_secs(30),
            "ready mesh backlog stalled"
        );
        std::thread::sleep(Duration::from_millis(8));
    }
    eprintln!(
        "ready_mesh_backlog count={count} drain_ms={} wait_ms={}",
        started.elapsed().as_millis(),
        stream.stats.max_mesh_queue_wait.as_millis()
    );
    assert_eq!(stream.stats.phase2_stages.mesh_jobs_completed, count as u64);
    assert_eq!(stream.stats.stale_mesh_jobs, 0);
}

/// A ready backlog supplies every worker, with room for results awaiting the next poll.
#[test]
fn mesh_admission_has_a_full_worker_wave() {
    let workers = 12;
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(workers)
        .build()
        .unwrap();
    let mut stream = lit_stream(1);
    for x in 0..workers * 3 {
        let key = SubChunkKey::new(1, x as i32 * 4, 0, 0);
        stream
            .store
            .commit_sub_chunk(key, super::uniform_sub_chunk(2))
            .unwrap();
        install_current_light(&mut stream, key, 0, 0, false);
        stream.mark_dirty_exact(key, Instant::now());
    }
    let dispatched = pool.install(|| stream.dispatch_mesh_jobs([0.0; 3], usize::MAX));
    assert!(
        dispatched >= workers,
        "only {dispatched} jobs admitted for {workers} workers"
    );
    for _ in 0..dispatched {
        stream.accept_mesh_completion(stream.mesh_rx.recv_timeout(Duration::from_secs(5)).unwrap());
    }
    assert_eq!(stream.admitted_mesh_jobs.load(Ordering::Acquire), 0);
}

/// Repeated light changes keep one successor while the cancelled predecessor retires.
#[test]
fn light_churn_supersedes_pending_mesh_in_place() {
    let mut stream = lit_stream(1);
    let key = SubChunkKey::new(1, 0, 0, 0);
    stream
        .store
        .commit_sub_chunk(key, super::uniform_sub_chunk(2))
        .unwrap();
    install_current_light(&mut stream, key, 0, 0, false);
    let revision = stream.mark_dirty_exact(key, Instant::now());
    stream.pending_mesh.remove(&key);
    stream.pending_mesh_scan.clear();
    stream.in_flight.insert(key, revision);
    let cancelled = Arc::new(AtomicBool::new(false));
    stream
        .mesh_cancellations
        .insert(key, Arc::clone(&cancelled));
    stream.mark_changed_light_mesh_dependents(key, [true; 6], Instant::now(), false);
    let successor = stream.pending_mesh[&key];
    for _ in 0..100 {
        stream.mark_changed_light_mesh_dependents(key, [true; 6], Instant::now(), true);
    }
    assert!(cancelled.load(Ordering::Acquire));
    assert_ne!(successor.revision, revision);
    assert_eq!(stream.pending_mesh[&key].revision, successor.revision);
    assert_eq!(stream.pending_mesh[&key].queued_at, successor.queued_at);
    assert!(stream.pending_mesh[&key].urgent);
    assert!(stream.pending_mesh_scan.len() <= 2);

    stream.in_flight.remove(&key);
    stream.mark_light_dirty_exact(key).unwrap();
    assert_eq!(stream.dispatch_mesh_jobs([0.0; 3], 1), 0);
    complete_one_light(&mut stream, [0.0; 3]);
    assert_eq!(stream.dispatch_mesh_jobs([0.0; 3], 1), 1);
    let completion = stream.mesh_rx.recv_timeout(Duration::from_secs(5)).unwrap();
    assert_eq!(completion.revision, successor.revision);
    stream.accept_mesh_completion(completion);
    assert_eq!(stream.stats.stale_mesh_jobs, 0);
    assert!(stream.pending_mesh.is_empty());
}

/// Exercises thousands of resident sub-chunks with roofs, emitters and repeated relighting.
#[test]
fn large_lighting_backlog_drains() {
    let mut stream = lit_stream(0);
    let range = vanilla_dimension_range(0).unwrap();
    let mut keys = Vec::new();
    for x in -6..=6 {
        for z in -6..=6 {
            for offset in 0..range.sub_chunk_count {
                let y = range.base_sub_chunk_y + offset as i32;
                let key = SubChunkKey::new(0, x, y, z);
                let id = if y == 19 && (x + z) % 2 == 0 {
                    Some(2)
                } else if y == 0 && (x + z) % 3 == 0 {
                    Some(1)
                } else if y == 8 {
                    Some(3)
                } else {
                    None
                };
                if let Some(id) = id {
                    stream
                        .store
                        .commit_sub_chunk(key, super::uniform_sub_chunk(id))
                        .unwrap();
                    stream.resident.insert(key);
                } else {
                    stream.record_known_air(key);
                }
                keys.push(key);
            }
        }
    }
    stream.mark_changed_sources(keys.iter().copied(), Instant::now());
    let started = Instant::now();
    let mut polls = Vec::new();
    for frame in 0..10_000 {
        if frame == 8 || frame == 16 {
            stream.mark_light_changed_sources(keys.iter().copied());
        }
        let poll = Instant::now();
        stream.poll([8.0, 81.62, 8.0], 64);
        polls.push(poll.elapsed().as_micros());
        acknowledge_mesh_changes(&mut stream);
        if frame % 100 == 0 {
            eprintln!(
                "backlog ms={} pending_mesh={} flight_mesh={} pending_light={} flight_light={} accepted={} stale_mesh={}",
                started.elapsed().as_millis(),
                stream.pending_mesh.len(),
                stream.in_flight.len(),
                stream.pending_light.len(),
                stream.in_flight_light.len(),
                stream.stats.accepted_light_jobs,
                stream.stats.stale_mesh_jobs
            );
        }
        if frame > 16
            && stream.pending_mesh.is_empty()
            && stream.in_flight.is_empty()
            && stream.pending_light.is_empty()
            && stream.in_flight_light.is_empty()
            && stream.staged_mesh_completions.is_empty()
        {
            polls.sort_unstable();
            eprintln!(
                "backlog drained subchunks={} ms={} poll_p99_us={} max_us={} stats={:?}",
                keys.len(),
                started.elapsed().as_millis(),
                polls[polls.len() * 99 / 100],
                polls.last().unwrap(),
                stream.stats()
            );
            assert!(keys.iter().all(|key| stream.light_is_current(*key)));
            return;
        }
        assert!(
            started.elapsed() < Duration::from_secs(60),
            "backlog failed to drain: {:?}",
            stream.stats()
        );
        std::thread::sleep(Duration::from_millis(2));
    }
    panic!("backlog did not drain");
}

/// Retires publications just as a completed upload acknowledgement does.
fn acknowledge_mesh_changes(stream: &mut WorldStream) {
    for change in stream.take_mesh_changes() {
        match change {
            WorldMeshChange::Upsert {
                key,
                generation,
                dirty_since,
                ..
            }
            | WorldMeshChange::Remove {
                key,
                generation,
                dirty_since,
                ..
            } => {
                stream.acknowledge_mesh_upload(key, generation, dirty_since, Instant::now());
            }
        }
    }
}
