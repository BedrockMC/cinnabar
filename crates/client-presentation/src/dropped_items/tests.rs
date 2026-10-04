use super::*;

fn stream(hashes: bool, internal: u32) -> WorldStream {
    let mut stream = WorldStream::new(protocol::WorldBootstrap {
        local_player_unique_id: 1,
        local_player_runtime_id: 1,
        dimension: 0,
        player_position: [0.0; 3],
        world_spawn_position: [0; 3],
        air_network_id: 0,
        block_network_ids_are_hashes: hashes,
    });
    stream.set_custom_block_ids(internal..internal + 1);
    if !hashes {
        stream.set_sequential_id_remap(assets::SequentialIdRemap::from_palette(
            vec![0, internal],
            internal + 1,
        ));
    }
    stream
}

#[test]
fn retained_items_and_falling_blocks_resolve_the_current_session_palette() {
    let visual = ItemVisualRoute::RetainedBlock {
        block_runtime_id: 1,
    };
    let falling = BlockEntityKind::Falling {
        block_runtime_id: 1,
    };
    for internal in [7, 6] {
        let stream = stream(false, internal);
        assert_eq!(
            item_block_id(&stream, visual),
            Some((NetworkIdMode::Sequential, internal))
        );
        assert_eq!(
            entity_block_id(&stream, &falling),
            Some((NetworkIdMode::Sequential, internal, FALLING_BLOCK_SCALE)),
        );
    }
    assert_eq!(
        visual,
        ItemVisualRoute::RetainedBlock {
            block_runtime_id: 1
        }
    );
    assert_eq!(
        falling,
        BlockEntityKind::Falling {
            block_runtime_id: 1
        }
    );
}

#[test]
fn retained_items_and_falling_blocks_preserve_high_bit_hashes() {
    let hash = 0x8000_0007_u32;
    let stream = stream(true, hash);
    let raw = i32::from_ne_bytes(hash.to_ne_bytes());
    assert!(raw < 0);
    assert_eq!(
        item_block_id(
            &stream,
            ItemVisualRoute::RetainedBlock {
                block_runtime_id: raw
            }
        ),
        Some((NetworkIdMode::Hashed, hash)),
    );
    assert_eq!(
        entity_block_id(
            &stream,
            &BlockEntityKind::Falling {
                block_runtime_id: raw
            }
        ),
        Some((NetworkIdMode::Hashed, hash, FALLING_BLOCK_SCALE)),
    );
}

#[test]
fn compiled_block_item_and_tnt_visuals_are_already_internal() {
    let stream = stream(false, 7);
    let visual = ItemVisualRoute::BlockItem(assets::BlockVisualId(1));
    assert_eq!(
        item_block_id(&stream, visual),
        Some((NetworkIdMode::Sequential, 1))
    );
    assert_eq!(
        entity_block_id(&stream, &BlockEntityKind::PrimedTnt { visual }),
        Some((NetworkIdMode::Sequential, 1, 1.0)),
    );
}
