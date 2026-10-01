use super::*;

#[test]
fn queued_mesh_cancellation_skips_superseded_geometry() {
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(1)
        .build()
        .unwrap();
    let mut stream = lit_stream(1);
    let key = SubChunkKey::new(1, 0, 0, 0);
    stream
        .store
        .commit_sub_chunk(key, super::uniform_sub_chunk(2))
        .unwrap();
    install_current_light(&mut stream, key, 0, 0, false);
    pool.install(|| {
        stream.mark_dirty_exact(key, Instant::now());
        assert_eq!(stream.dispatch_mesh_jobs([0.0; 3], 1), 1);
        stream.mark_dirty_exact(key, Instant::now());
    });
    let completion = stream.mesh_rx.recv_timeout(Duration::from_secs(2)).unwrap();
    assert!(completion.mesh.is_empty());
    assert_eq!(stream.admitted_mesh_jobs.load(Ordering::Acquire), 1);
    stream.accept_mesh_completion(completion);
    assert_eq!(stream.admitted_mesh_jobs.load(Ordering::Acquire), 0);
    assert!(stream.mesh_changes.is_empty());
    assert_eq!(stream.stats().stale_mesh_jobs, 1);
}

/// Times a turn while a large mesh backlog is waiting for source readiness.
#[test]
#[ignore = "offline scheduler timing fixture"]
fn scheduler_turn_timing() {
    let mut stream = lit_stream(1);
    for x in 0..20_000 {
        let key = SubChunkKey::new(1, x, 0, 0);
        stream.resident.insert(key);
        stream.mark_dirty_exact(key, Instant::now());
    }
    let mut samples = Vec::new();
    for x in 0..31 {
        let started = Instant::now();
        assert_eq!(stream.dispatch_mesh_jobs([x as f32 * 16.0, 0.0, 0.0], 1), 0);
        samples.push(started.elapsed().as_micros());
    }
    samples.remove(0);
    samples.sort_unstable();
    println!(
        "scheduler_turn pending={} median_us={} p95_us={}",
        stream.pending_mesh.len(),
        samples[15],
        samples[28]
    );
}
