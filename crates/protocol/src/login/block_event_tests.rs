//! `BlockEvent` ingress: container cues reach the world stream with their dimension.

use bytes::Buf;
use jolyne::raw::decode_packet_raw;
use valentine::bedrock::context::BedrockSession;
use valentine::bedrock::version::v1_26_44::{BlockEventPacket, BlockPos};

use super::*;
use crate::{BlockEventEvent, WorldEvent};

#[test]
fn block_event_packets_normalize_with_the_current_dimension() {
    let session = BedrockSession { shield_item_id: 0 };
    let packet: Packet = BlockEventPacket {
        block_position: BlockPos { x: 4, y: 70, z: -9 },
        event_type: 1,
        event_value: 2,
    }
    .into();
    let mut batch = crate::encode(&packet, &session).expect("encode block event");
    batch.advance(1);
    let raw = decode_packet_raw(&mut batch).expect("raw block event");

    let event = decode_world_raw_with(raw, 1, |raw| raw.decode(&session))
        .expect("decode block event")
        .expect("world event");

    assert_eq!(
        event,
        WorldEvent::BlockEvent(BlockEventEvent {
            dimension: 1,
            position: [4, 70, -9],
            event_type: 1,
            event_value: 2,
        })
    );
}
