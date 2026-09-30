//! Normalizers for block-side packets: map pixel updates, sign editor requests and block events.

use valentine::bedrock::version::v1_26_51::{
    BlockEventPacket, ClientboundMapItemDataPacket, OpenSignPacket,
};

use super::events::{BlockEventEvent, MAP_IMAGE_SIDE, MapDataEvent, OpenSignEvent};

/// The pixel rectangle of a map update; `None` for a partial packet (no pixels) or a rectangle
/// outside the fixed image or with the wrong pixel count, which are semantic skips.
pub(super) fn normalize_map_data(packet: &ClientboundMapItemDataPacket) -> Option<MapDataEvent> {
    let (Some(width), Some(height), Some(start_x), Some(start_y), Some(pixels)) = (
        packet.width,
        packet.height,
        packet.start_x,
        packet.start_y,
        packet.pixels.as_deref(),
    ) else {
        return None;
    };
    let (Ok(width), Ok(height), Ok(start_x), Ok(start_y)) = (
        u32::try_from(width),
        u32::try_from(height),
        u32::try_from(start_x),
        u32::try_from(start_y),
    ) else {
        return None;
    };
    let fits = width != 0
        && height != 0
        && width.saturating_add(start_x) <= MAP_IMAGE_SIDE
        && height.saturating_add(start_y) <= MAP_IMAGE_SIDE
        && pixels.len() as u64 == u64::from(width) * u64::from(height);
    fits.then(|| MapDataEvent {
        map_id: packet.map_id.actor_unique_id,
        start_x,
        start_y,
        width,
        height,
        pixels: pixels.into(),
    })
}

pub(super) fn normalize_open_sign(packet: &OpenSignPacket, dimension: i32) -> OpenSignEvent {
    OpenSignEvent {
        dimension,
        position: [packet.pos.x, packet.pos.y, packet.pos.z],
        front: packet.is_front_side,
    }
}

pub(super) fn normalize_block_event(packet: &BlockEventPacket, dimension: i32) -> BlockEventEvent {
    BlockEventEvent {
        dimension,
        position: [
            packet.block_position.x,
            packet.block_position.y,
            packet.block_position.z,
        ],
        event_type: packet.event_type,
        event_value: packet.event_value,
    }
}
