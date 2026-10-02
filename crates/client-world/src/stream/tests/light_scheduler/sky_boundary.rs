use super::*;

#[test]
fn transparent_occupied_top_cell_receives_sky_and_opaque_top_cell_filters_it() {
    for (runtime_id, expected) in [(3, 15), (2, 0)] {
        let mut stream = lit_stream(0);
        let top = SubChunkKey::new(0, 0, 19, 0);
        stream
            .store
            .commit_sub_chunk(top, super::uniform_sub_chunk(runtime_id))
            .unwrap();
        stream.resident.insert(top);
        stream.mark_changed(top, Instant::now());
        complete_one_light(&mut stream, [8.0, 312.0, 8.0]);
        assert_eq!(
            stream
                .light_store
                .light(top)
                .unwrap()
                .get(LightChannel::Sky, 8, 15, 8),
            Some(expected)
        );
    }
}

#[test]
fn taller_columns_dispatch_the_highest_pending_source_before_its_dependency() {
    let mut stream = lit_stream(0);
    let range = vanilla_dimension_range(0).unwrap();
    let vanilla_top = range.base_sub_chunk_y + range.sub_chunk_count as i32 - 1;
    let lower = SubChunkKey::new(0, 0, vanilla_top, 0);
    let upper = SubChunkKey::new(0, 0, vanilla_top + 1, 0);
    for key in [lower, upper] {
        stream
            .store
            .commit_sub_chunk(key, super::uniform_sub_chunk(3))
            .unwrap();
        stream.resident.insert(key);
        stream.mark_changed(key, Instant::now());
    }
    assert_eq!(
        stream.highest_pending_light_in_column(lower).unwrap().0,
        upper
    );
    assert_eq!(
        stream.light_block_snapshot(upper).overworld_top_y,
        Some(upper.y * 16 + 15)
    );
    complete_one_light(&mut stream, [8.0, upper.y as f32 * 16.0 + 8.0, 8.0]);
    assert!(stream.light_is_current(upper));
    settle_light(&mut stream, [8.0, lower.y as f32 * 16.0 + 8.0, 8.0]);
    assert!(stream.light_is_current(lower));
}
