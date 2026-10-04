use valentine::bedrock::version::v1_26_51::{
    ActorRuntimeId, ChangeDimensionPacket, EnumsPlayerActionType, EnumsPlayerRespawnState,
    EnumsServerboundLoadingScreenPacketType, PlayerActionPacket, RespawnPacket,
    ServerboundLoadingScreenPacket,
};

use crate::Packet;

use super::{ChangeDimensionEvent, RespawnEvent, WorldEvent};

pub(super) fn normalize_change_dimension(packet: &ChangeDimensionPacket) -> ChangeDimensionEvent {
    ChangeDimensionEvent {
        dimension: packet.dimension_id.value,
        position: [packet.position.x, packet.position.y, packet.position.z],
        respawn: packet.respawn,
        loading_screen_id: packet.loading_screen_id,
    }
}

pub(super) fn normalize_dimension_action(packet: &PlayerActionPacket) -> Option<WorldEvent> {
    (packet.action == EnumsPlayerActionType::Changedimensionack).then_some(
        WorldEvent::DimensionChangeAck {
            runtime_id: packet.player_runtime_id.actor_runtime_id,
        },
    )
}

pub(super) fn normalize_respawn(packet: &RespawnPacket) -> RespawnEvent {
    RespawnEvent {
        position: [packet.position.x, packet.position.y, packet.position.z],
        state: match packet.state {
            EnumsPlayerRespawnState::Searchingforspawn => 0,
            EnumsPlayerRespawnState::Readytospawn => 1,
            EnumsPlayerRespawnState::Clientreadytospawn => 2,
            EnumsPlayerRespawnState::Unknown(value) => value,
        },
        runtime_entity_id: packet.player_runtime_id.actor_runtime_id,
    }
}

/// Completes the local dimension transfer after the server wait and loaded-area checks.
#[must_use]
pub fn dimension_change_ack_packet(runtime_id: u64) -> Packet {
    PlayerActionPacket {
        player_runtime_id: ActorRuntimeId {
            actor_runtime_id: runtime_id,
        },
        action: EnumsPlayerActionType::Changedimensionack,
        ..Default::default()
    }
    .into()
}

/// Preserves the server's optional loading identifier through the start/end pair.
#[must_use]
pub fn dimension_loading_screen_packet(loading_screen_id: Option<u32>, started: bool) -> Packet {
    ServerboundLoadingScreenPacket {
        loading_screen_packet_type: if started {
            EnumsServerboundLoadingScreenPacketType::Startloadingscreen
        } else {
            EnumsServerboundLoadingScreenPacketType::Endloadingscreen
        },
        loading_screen_id,
    }
    .into()
}
