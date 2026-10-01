/// Connection states in the real registry must reach both runtime-ID maps.
#[test]
fn registry_connections_extend_fence_and_pane_colliders() {
    let breg = include_bytes!("../../../../crates/assets/data/block-registry-v2193.bin");
    let protocol = crate::asset_startup::active_content_registry_protocol();
    let records = read_registry_for_protocol(breg, protocol).unwrap();
    let preg = synthetic_preg(breg, &records);
    let registries = PhysicsCollisionRegistries::from_assets(breg, &records, &preg, protocol).unwrap();
    for (name, inset, height, end) in [
        ("minecraft:oak_fence", 0.375, 1.5, 0.625),
        ("minecraft:glass_pane", 0.4375, 1.0, 0.5),
        ("minecraft:iron_bars", 0.4375, 1.0, 0.5),
    ] {
        let record = records.iter().find(|record| {
            if record.name.as_ref() != name { return false; }
            let state: serde_json::Value = serde_json::from_str(&record.canonical_state).unwrap();
            state["minecraft:connection_north"]["value"] == 1
                && ["south", "east", "west"].into_iter().all(|direction| {
                    state[format!("minecraft:connection_{direction}")]["value"] == 0
                })
        }).unwrap();
        let expected = [Aabb::new(Vec3::new(inset, 0.0, 0.0), Vec3::new(1.0 - inset, height, end))];
        for (mode, id) in [(NetworkIdMode::Sequential, record.sequential_id), (NetworkIdMode::Hashed, record.network_hash)] {
            assert_eq!(registries.registry(mode).collision_shapes(id).unwrap(), expected, "{name} {mode:?}");
            let air = records.iter().find(|record| record.name.as_ref() == "minecraft:air").unwrap();
            let air_id = match mode { NetworkIdMode::Sequential => air.sequential_id, NetworkIdMode::Hashed => air.network_hash };
            let mut store = world::ChunkStore::new();
            let sub_chunk = world::SubChunkKey::new(0, 0, 0, 0);
            store.mark_sub_chunk_loaded(sub_chunk).unwrap();
            store.update_block(sub_chunk, world::BlockUpdate::new(8, 8, 8, 0, id), air_id).unwrap();
            let world = sim::PaletteWorld::new(&store, registries.registry(mode), 0);
            let arm = world.collision_boxes(Aabb::new(Vec3::new(8.45, 8.0, 8.01), Vec3::new(8.55, 9.0, 8.1))).unwrap();
            assert_eq!(arm.value, [expected[0].translated(Vec3::new(8.0, 8.0, 8.0))]);
        }
    }
}
