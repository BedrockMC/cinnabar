use super::*;

/// Creates a confirmed player grid without a server movement echo.
fn stream() -> WorldStream {
    let mut stream = WorldStream::new(WorldBootstrap {
        local_player_unique_id: 1,
        dimension: 0,
        local_player_runtime_id: 1,
        player_position: [0.5, 70.0, 0.5],
        world_spawn_position: [0, 70, 0],
        air_network_id: protocol::SEQUENTIAL_AIR_NETWORK_ID,
        block_network_ids_are_hashes: false,
    });
    stream.submit(1, WorldEvent::ChunkRadiusUpdated(2)).unwrap();
    stream
}

/// Publishes a local physics position with the stream's current ownership identity.
fn retain(stream: &mut WorldStream, position: [f32; 3]) -> bool {
    stream.retain_for_local_player(
        stream.authority.actor_session_id(),
        stream.current_dimension(),
        stream.form_dimension_epoch(),
        position,
    )
}

/// Populates the exact player grid, retaining overlapping columns from earlier moves.
fn populate_view(stream: &mut WorldStream, center: [i32; 2]) -> usize {
    let radius = stream.chunk_radius.unwrap();
    let reach = radius + world::CHUNK_VIEW_SLACK;
    let mut count = 0;
    for x in center[0] - reach..=center[0] + reach {
        for z in center[1] - reach..=center[1] + reach {
            if chunk_in_view(radius, [x, z], center) {
                let column = ChunkKey::new(stream.current_dimension(), x, z);
                stream.loaded_columns.insert(column);
                stream.record_known_air(SubChunkKey::from_chunk(column, 64));
                count += 1;
            }
        }
    }
    count
}

#[test]
fn ordinary_local_movement_bounds_residency_without_server_echoes() {
    let mut stream = stream();
    let server_position = stream.resolved_server_position();
    let publisher = stream.publisher.center;
    for center in 0..64 {
        let count = populate_view(&mut stream, [center, 0]);
        let position = [center as f32 * 16.0 + 0.5, 70.0, 0.5];
        assert_eq!(retain(&mut stream, position), center != 0);
        assert_eq!(stream.loaded_columns.len(), count);
        assert_eq!(stream.resident.len(), count);
        assert!(stream.tracked_columns().iter().all(|key| {
            chunk_in_view(stream.chunk_radius.unwrap(), [key.x, key.z], [center, 0])
        }));
        assert!(stream.column_is_data_interesting(ChunkKey::new(0, center, 0)));
        let generation = stream.connectivity_generation();
        assert!(!retain(&mut stream, [position[0] + 1.0, 70.0, 0.5]));
        assert_eq!(stream.connectivity_generation(), generation);
    }
    assert_eq!(stream.resolved_server_position(), server_position);
    assert_eq!(stream.publisher.center, publisher);
}

#[test]
fn local_grid_eviction_cancels_out_of_range_requests() {
    let mut stream = stream();
    let old = ChunkKey::new(0, 0, 0);
    stream.enqueue_request(old, 0, 1, None);
    assert!(stream.requests.is_expected(SubChunkKey::from_chunk(old, 0)));
    assert_ne!(stream.pending_request_count(), 0);
    assert!(retain(&mut stream, [320.0, 70.0, 0.0]));
    assert!(!stream.requests.requested.contains_key(&old));
    assert_eq!(stream.pending_request_count(), 0);
    assert!(stream.pop_next_request().is_none());
}

#[test]
fn local_retention_rejects_stale_owners_and_nonfinite_positions() {
    let mut stream = stream();
    let session = stream.authority.actor_session_id();
    let epoch = stream.form_dimension_epoch();
    for (owner, dimension, observed_epoch, position) in [
        (session.wrapping_add(1), 0, epoch, [320.0, 70.0, 0.0]),
        (session, 1, epoch, [320.0, 70.0, 0.0]),
        (session, 0, epoch.wrapping_add(1), [320.0, 70.0, 0.0]),
        (session, 0, epoch, [f32::NAN, 70.0, 0.0]),
        (session, 0, epoch, [0.0, f32::INFINITY, 0.0]),
    ] {
        assert!(!stream.retain_for_local_player(owner, dimension, observed_epoch, position));
        assert_eq!(stream.local_player_chunk, None);
    }
    assert!(retain(&mut stream, [-0.5, 70.0, -16.5]));
    assert_eq!(stream.local_player_chunk, Some(ChunkKey::new(0, -1, -2)));
    assert!(!retain(&mut stream, [-15.5, 70.0, -31.5]));
}

#[test]
fn server_spatial_commits_replace_the_local_retention_center() {
    let mut stream = stream();
    retain(&mut stream, [320.0, 70.0, 0.0]);
    stream
        .submit(
            2,
            WorldEvent::MovePlayer(MovePlayerEvent {
                runtime_id: 1,
                position: [-0.5, 70.0, -16.5],
                ..Default::default()
            }),
        )
        .unwrap();
    assert_eq!(stream.local_player_chunk, None);
    assert_eq!(stream.last_retention_center, Some(ChunkKey::new(0, -1, -2)));
    retain(&mut stream, [320.0, 70.0, 0.0]);
    let previous_epoch = stream.form_dimension_epoch();
    stream
        .submit(
            3,
            WorldEvent::ChangeDimension(ChangeDimensionEvent {
                dimension: 1,
                position: [0.0, 70.0, 0.0],
                ..Default::default()
            }),
        )
        .unwrap();
    assert_eq!(stream.local_player_chunk, None);
    assert!(!stream.retain_for_local_player(
        stream.authority.actor_session_id(),
        0,
        previous_epoch,
        [320.0, 70.0, 0.0],
    ));
}
