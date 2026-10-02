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
