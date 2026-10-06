use bytes::{BufMut, BytesMut};
use jolyne::raw::decode_packet_raw;
use valentine::bedrock::context::BedrockSession;
use valentine::bedrock::version::v1_26_51::{
    ClientCacheMissResponsePacket, LevelChunkPacket, LevelChunkPacketPayloadSubChunkMetadata,
    McpePacketName, MissingBlobData, SetTimePacket,
};

use super::*;
use crate::{BlobCacheReady, WorldEvent};

/// Builds a cache-enabled LevelChunk carrying exactly the given blob hashes.
///
/// 1.26.40 writes the hashes unconditionally and signals cache participation
/// with `cache_enabled`, so `blobs: Some(..)` has no direct equivalent.
fn cached_level_chunk(hashes: Vec<u64>) -> LevelChunkPacket {
    let subchunks_count = u32::try_from(hashes.len().saturating_sub(1)).expect("fixture count");
    LevelChunkPacket {
        subchunks_count,
        cache_enabled: true,
        cache_metadata: hashes
            .into_iter()
            .map(|blob_id| LevelChunkPacketPayloadSubChunkMetadata { blob_id })
            .collect(),
        ..Default::default()
    }
}

fn raw_packet(id: McpePacketName, body: &[u8]) -> jolyne::raw::RawPacket {
    let mut payload = BytesMut::new();
    wire::write_var_u32(&mut payload, id as u32);
    payload.put_slice(body);
    let mut frame = BytesMut::new();
    wire::write_var_u32(&mut frame, payload.len() as u32);
    frame.put_slice(&payload);
    decode_packet_raw(&mut frame.freeze()).expect("raw packet")
}

#[test]
fn transfer_resets_pending_cache_transactions_but_change_dimension_is_ordered() {
    let cache = ClientBlobCache::default();
    let mut resolver = BlobCacheResolver::new(cache);
    let missing = crate::client_blob_hash(b"missing");
    resolver
        .accept_cached_packet(cached_level_chunk(vec![missing]).into())
        .expect("pending cached column");

    assert!(
        !reset_cache_for_immediate_boundary(&mut resolver, McpePacketName::ChangeDimensionPacket)
            .expect("change dimension does not reset immediately")
    );
    assert_eq!(resolver.stats().pending_transactions, 1);
    assert!(
        reset_cache_for_immediate_boundary(&mut resolver, McpePacketName::TransferPacket)
            .expect("transfer preserves rollback recovery")
    );
    assert_eq!(resolver.stats().pending_transactions, 0);
    assert_eq!(resolver.stats().pending_resets, 1);
    assert!(matches!(
        resolver.pop_ready(),
        Some(BlobCacheReady::WorldEvent(WorldEvent::ChunkResync(_)))
    ));
}

#[test]
fn unrelated_world_semantic_skip_does_not_recover_pending_cached_terrain() {
    let cache = ClientBlobCache::default();
    let missing_payload = b"late terrain blob";
    let missing = crate::client_blob_hash(missing_payload);
    let mut resolver = BlobCacheResolver::new(cache);
    resolver
        .accept_cached_packet(cached_level_chunk(vec![missing]).into())
        .expect("pending cached column");
    assert_eq!(resolver.stats().pending_transactions, 1);

    let mut world_skips = 0;
    skip_semantic_world_error(
        ProtocolError::World(crate::WorldPacketError::Ui(
            crate::UiPacketError::UnknownEnum {
                kind: "text category",
                value: 3,
            },
        )),
        &mut world_skips,
    )
    .expect("unrelated semantic rejection is skipped");
    assert_eq!(world_skips, 1);
    assert_eq!(resolver.stats().pending_transactions, 1);

    resolver
        .accept_miss_response(ClientCacheMissResponsePacket {
            missing_blobs: vec![MissingBlobData {
                blob_id: missing,
                blob_data: missing_payload.to_vec(),
            }],
        })
        .expect("late cache miss response resolves normally");
    assert_eq!(resolver.stats().pending_transactions, 0);
    assert!(matches!(
        resolver.pop_ready(),
        Some(BlobCacheReady::Packet(Packet {
            data: McpePacketData::LevelChunkPacket(_),
            ..
        }))
    ));
}

#[test]
fn fatal_session_reset_clears_pending_without_clearing_verified_entries() {
    let cache = ClientBlobCache::default();
    let verified = cache
        .insert(b"verified terrain")
        .expect("seed verified blob");
    let missing = crate::client_blob_hash(b"missing terrain");
    let mut resolver = BlobCacheResolver::new(cache.clone());
    resolver
        .accept_cached_packet(cached_level_chunk(vec![missing]).into())
        .expect("pending cached column");

    resolver.reset_pending();

    assert_eq!(resolver.stats().pending_transactions, 0);
    assert!(cache.contains(verified));
}

#[test]
fn fast_transfer_arm_is_consumed_only_after_a_chunk_candidate_decodes() {
    let missing = crate::client_blob_hash(b"old-backend-missing");
    let mut resolver = BlobCacheResolver::new(ClientBlobCache::default());
    resolver
        .accept_cached_packet(cached_level_chunk(vec![missing]).into())
        .expect("old unresolved transaction");
    resolver.arm_fast_transfer_reset();

    let session = BedrockSession { shield_item_id: 0 };
    let malformed = raw_packet(McpePacketName::LevelChunkPacket, &[0xff]);
    assert!(malformed.decode(&session).is_err());
    assert_eq!(resolver.stats().pending_transactions, 1);

    let ordinary: crate::Packet = SetTimePacket { time: 7 }.into();
    assert!(
        !reset_blob_cache_for_decoded_candidate(&mut resolver, &ordinary)
            .expect("ordinary decoded packet is not a candidate")
    );
    assert_eq!(resolver.stats().pending_transactions, 1);

    let candidate: crate::Packet = LevelChunkPacket {
        subchunks_count: 0,
        cache_enabled: false,
        ..Default::default()
    }
    .into();
    assert!(
        reset_blob_cache_for_decoded_candidate(&mut resolver, &candidate)
            .expect("successfully decoded candidate consumes the arm")
    );
    assert_eq!(resolver.stats().pending_transactions, 0);
    assert!(
        !reset_blob_cache_for_decoded_candidate(&mut resolver, &candidate)
            .expect("arm is one-shot")
    );
}

#[test]
fn malformed_cached_chunk_wire_remains_a_fatal_session_error() {
    let session = BedrockSession { shield_item_id: 0 };
    let malformed = raw_packet(McpePacketName::LevelChunkPacket, &[0xff]);

    let error = decode_world_raw_with(malformed, 0, |raw| raw.decode(&session))
        .expect_err("truncated cached LevelChunk wire must fail closed");

    assert!(matches!(error, ProtocolError::Session(_)));
}

#[test]
fn malformed_cache_miss_response_wire_remains_a_fatal_decode_error() {
    let session = BedrockSession { shield_item_id: 0 };
    let truncated = raw_packet(McpePacketName::ClientCacheMissResponsePacket, &[0x01]);

    assert!(
        truncated.decode(&session).is_err(),
        "a declared blob without its hash and payload must fail closed in raw decode"
    );
}
