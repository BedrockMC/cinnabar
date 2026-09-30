use super::*;

fn fixture() -> WorldStream {
    let mut stream = WorldStream::new_with_assets(
        WorldBootstrap {
            local_player_unique_id: 1,
            local_player_runtime_id: 1,
            dimension: 0,
            player_position: [0.0; 3],
            world_spawn_position: [0; 3],
            air_network_id: 2,
            block_network_ids_are_hashes: false,
        },
        Arc::new(non_default_air_runtime_assets()),
        [0.0; 3],
        None,
    );
    let mut payload = vec![9, 1, (-4_i8) as u8, 1, 0];
    payload.extend(biome_payload(0, 1));
    stream
        .submit(
            1,
            WorldEvent::LevelChunk(LevelChunkEvent {
                dimension: 0,
                x: 0,
                z: 0,
                mode: LevelChunkMode::Inline { count: 1 },
                payload,
            }),
        )
        .unwrap();
    complete_pending_decode_jobs(&mut stream);
    stream
}

fn server_update(stream: &mut WorldStream, sequence: u64, position: [i32; 3], network_id: u32) {
    stream
        .submit(
            sequence,
            WorldEvent::BlockUpdates(vec![BlockUpdateEvent {
                dimension: 0,
                position,
                layer: 0,
                network_id,
            }]),
        )
        .unwrap();
}

fn block(stream: &WorldStream, [x, y, z]: [i32; 3]) -> Option<u32> {
    stream
        .store
        .sub_chunk(SubChunkKey::new(
            0,
            x.div_euclid(16),
            y.div_euclid(16),
            z.div_euclid(16),
        ))
        .and_then(|sub_chunk| {
            sub_chunk.runtime_id(
                0,
                x.rem_euclid(16) as u8,
                y.rem_euclid(16) as u8,
                z.rem_euclid(16) as u8,
            )
        })
}

/// A prediction lands at once and the server's later word replaces it.
#[test]
fn a_prediction_commits_immediately_and_a_server_update_overrides_it() {
    let mut stream = fixture();
    let air = stream.air_block_id();
    assert!(stream.predict_block([3, -64, 3], 0, air));
    assert_eq!(block(&stream, [3, -64, 3]), Some(air));
    server_update(&mut stream, 2, [3, -64, 3], 0);
    complete_pending_decode_jobs(&mut stream);
    assert_eq!(
        block(&stream, [3, -64, 3]),
        Some(0),
        "the server rolls it back"
    );
}

#[test]
fn a_prediction_outside_authoritative_data_is_refused() {
    let mut stream = fixture();
    let air = stream.air_block_id();
    assert!(!stream.predict_block([40, -64, 3], 0, air));
    assert!(!stream.predict_block([3, 400, 3], 0, air));
    assert!(
        stream.predict_block([3, 200, 3], 0, 1),
        "a loaded column is known air"
    );
}

/// A server batch received before the prediction must not erase it on commit.
#[test]
fn a_prediction_survives_an_in_flight_batch_received_before_it() {
    let mut stream = fixture();
    let air = stream.air_block_id();
    server_update(&mut stream, 2, [5, -64, 5], 1);
    assert!(stream.predict_block([3, -64, 3], 0, air));
    assert!(stream.predict_block([5, -64, 5], 0, air));
    complete_pending_decode_jobs(&mut stream);
    assert_eq!(block(&stream, [3, -64, 3]), Some(air));
    assert_eq!(block(&stream, [5, -64, 5]), Some(air));
    server_update(&mut stream, 3, [5, -64, 5], 1);
    complete_pending_decode_jobs(&mut stream);
    assert_eq!(block(&stream, [5, -64, 5]), Some(1), "a later batch wins");
}
