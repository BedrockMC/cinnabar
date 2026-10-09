use super::*;

/// Queues `count` inline air columns with their decodes already completed.
fn stream_with_decoded_columns(count: u64) -> WorldStream {
    let mut stream = block_entity_visual_stream();
    for sequence in 1..=count {
        stream
            .submit(sequence, inline_air_event(sequence as i32))
            .unwrap();
    }
    while let Some(queued) = stream.pending_decode.pop_front() {
        stream.in_flight_decode_jobs += 1;
        let completion = queued.job.run(queued.queued_at);
        stream.decode_tx.send(completion).unwrap();
    }
    stream
}

/// A reclaim request stops a poll after its one guaranteed heavy commit even with every
/// decode accepted and a whole slice left, which bounds the frame's wait for its stream.
#[test]
fn raised_service_yield_limits_a_poll_to_its_guaranteed_heavy_commit() {
    let mut stream = stream_with_decoded_columns(8);
    while let Ok(completion) = stream.decode_rx.try_recv() {
        stream.accept_decode_completion(completion);
    }
    stream.service_yield = Some(Arc::new(AtomicBool::new(true)));
    stream.frame_deadline = Some(Instant::now() + Duration::from_secs(60));
    stream.poll([0.0; 3], 0);
    assert_eq!(stream.committed_sequence(), 1);

    stream.service_yield = Some(Arc::new(AtomicBool::new(false)));
    stream.frame_deadline = Some(Instant::now() + Duration::from_secs(60));
    let report = stream.poll([0.0; 3], 0);
    assert_eq!(
        stream.committed_sequence(),
        8,
        "an unraised flag leaves the slice intact"
    );
    assert_eq!(report.commit_steps, 7, "the report counts the progress");
}

/// After a service window as long as the frame's allocation, the frame's poll accepts decode
/// results but leaves chunk-data commits to the service; a short window keeps the frame's
/// usual guaranteed commit, so a service starved of time never stalls terrain.
#[test]
fn frame_poll_leaves_chunk_data_to_a_service_with_a_full_window() {
    let mut stream = stream_with_decoded_columns(4);
    stream.between_frames_service = true;
    stream.service_window = stream.poll_budget;
    stream.begin_frame_work();
    let report = stream.poll([0.0; 3], 0);
    assert!(report.decoded_results > 0);
    assert_eq!(stream.committed_sequence(), 0);

    stream.service_window = Duration::ZERO;
    stream.frame_deadline = Some(Instant::now());
    stream.poll([0.0; 3], 0);
    assert_eq!(
        stream.committed_sequence(),
        1,
        "the frame's guaranteed heavy commit"
    );
}

/// With no frame-thread poll at all, the service commits decoded terrain between frames and
/// hands the stream back with that work recorded.
#[test]
fn service_commits_decoded_terrain_without_a_frame_poll() {
    let mut service = WorldStreamService::spawn().unwrap();
    let mut stream = stream_with_decoded_columns(4);
    let mut decoded = 0;
    let started = Instant::now();
    while stream.committed_sequence() < 4 {
        assert!(
            started.elapsed() < Duration::from_secs(10),
            "service made no progress"
        );
        service.launch(stream, [0.0; 3], 0);
        assert!(service.is_servicing());
        std::thread::sleep(Duration::from_millis(1));
        let serviced = service.reclaim().expect("the service holds the stream");
        assert!(!service.is_servicing());
        assert!(serviced.held >= serviced.busy);
        decoded += serviced.report.decoded_results;
        stream = serviced.stream;
    }
    assert_eq!(decoded, 4);
    assert!(
        service.reclaim().is_none(),
        "nothing is held once reclaimed"
    );
}

/// Dropping the service while it holds a stream drops that stream before returning, rather
/// than leaving it on a detached thread past application shutdown.
#[test]
fn dropping_a_servicing_service_drops_its_stream() {
    let mut service = WorldStreamService::spawn().unwrap();
    let stream = stream_with_decoded_columns(2);
    let witness = Arc::clone(&stream.admitted_mesh_jobs);
    service.launch(stream, [0.0; 3], 0);
    drop(service);
    assert_eq!(Arc::strong_count(&witness), 1);
}
