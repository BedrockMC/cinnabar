//! Passive, bounded ability evidence. This module does not resolve permissions.
//! Wire fields follow protocol 2168 and the pinned gophertunnel
//! `minecraft/protocol/ability.go` at 3d9f4b7a4ac0.
use std::sync::Arc;

use bytes::Buf;
use valentine::bedrock::{
    codec::{BedrockCodec, VarUInt},
    error::DecodeError,
    version::v1_26_51::{
        EnumsCommandPermissionLevel, EnumsPlayerPermissionLevel, SerializedAbilitiesData,
    },
};

use crate::ProtocolError;

/// Client retention policy, not a protocol or vanilla layer limit.
pub const MAX_ABILITY_LAYERS: usize = 32;
const LAYER_WIRE_BYTES: usize = 22;

/// An uninterpreted layer. Float bits preserve non-finite values without using them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AbilityLayerEvidence {
    pub layer_type: u16,
    pub abilities: u32,
    pub values: u32,
    pub fly_speed_bits: u32,
    pub vertical_fly_speed_bits: u32,
    pub walk_speed_bits: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AbilityLayersEvidence {
    /// Packet order is significant evidence, not a precedence rule.
    Received(Arc<[AbilityLayerEvidence]>),
    /// A correctly framed update exceeds the client retention policy.
    Unavailable { declared_layers: u32 },
}

/// One received update, addressed by persistent actor identity, not runtime ID.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AbilitiesUpdate {
    pub actor_unique_id: i64,
    pub player_permission: i8,
    pub command_permission: u8,
    pub layers: AbilityLayersEvidence,
}

/// Decode the complete UpdateAbilities body without materializing an unbounded collection.
/// Unknown discriminants, masks and float bits are inert evidence, not wire errors.
pub fn decode_abilities_update(mut body: &[u8]) -> Result<AbilitiesUpdate, ProtocolError> {
    if body.len() < 10 {
        return Err(DecodeError::UnexpectedEof {
            needed: 10,
            available: body.len(),
        }
        .into());
    }
    let actor_unique_id = body.get_i64_le();
    let player_permission = body.get_i8();
    let command_permission = body.get_u8();
    let count = VarUInt::decode(&mut body, ())?.0;
    let required = usize::try_from(count)
        .ok()
        .and_then(|count| count.checked_mul(LAYER_WIRE_BYTES))
        .ok_or(DecodeError::VarIntTooLarge)?;
    if required > body.len() {
        return Err(ProtocolError::TruncatedPacket {
            declared: required,
            available: body.len(),
        });
    }
    if required < body.len() {
        return Err(ProtocolError::TrailingPacketBytes {
            remaining: body.len() - required,
        });
    }
    let layers = if count as usize > MAX_ABILITY_LAYERS {
        AbilityLayersEvidence::Unavailable {
            declared_layers: count,
        }
    } else {
        let mut layers = Vec::with_capacity(count as usize);
        for _ in 0..count {
            layers.push(AbilityLayerEvidence {
                layer_type: body.get_u16_le(),
                abilities: body.get_u32_le(),
                values: body.get_u32_le(),
                fly_speed_bits: body.get_u32_le(),
                vertical_fly_speed_bits: body.get_u32_le(),
                walk_speed_bits: body.get_u32_le(),
            });
        }
        AbilityLayersEvidence::Received(layers.into())
    };
    Ok(AbilitiesUpdate {
        actor_unique_id,
        player_permission,
        command_permission,
        layers,
    })
}

pub(crate) fn normalize_abilities(data: SerializedAbilitiesData) -> AbilitiesUpdate {
    let player_permission = match data.player_permissions {
        EnumsPlayerPermissionLevel::Visitor => 0,
        EnumsPlayerPermissionLevel::Member => 1,
        EnumsPlayerPermissionLevel::Operator => 2,
        EnumsPlayerPermissionLevel::Custom => 3,
        EnumsPlayerPermissionLevel::Unknown(value) => value,
    };
    let command_permission = match data.command_permissions {
        EnumsCommandPermissionLevel::Any => 0,
        EnumsCommandPermissionLevel::Gamedirectors => 1,
        EnumsCommandPermissionLevel::Admin => 2,
        EnumsCommandPermissionLevel::Host => 3,
        EnumsCommandPermissionLevel::Owner => 4,
        EnumsCommandPermissionLevel::Internal => 5,
        EnumsCommandPermissionLevel::Unknown(value) => value,
    };
    let layers = if data.layers.len() > MAX_ABILITY_LAYERS {
        AbilityLayersEvidence::Unavailable {
            declared_layers: u32::try_from(data.layers.len()).unwrap_or(u32::MAX),
        }
    } else {
        AbilityLayersEvidence::Received(
            data.layers
                .into_iter()
                .map(|layer| AbilityLayerEvidence {
                    layer_type: layer.serialized_layer,
                    abilities: layer.abilities_set,
                    values: layer.ability_values,
                    fly_speed_bits: layer.fly_speed.to_bits(),
                    vertical_fly_speed_bits: layer.vertical_fly_speed.to_bits(),
                    walk_speed_bits: layer.walk_speed.to_bits(),
                })
                .collect(),
        )
    };
    AbilitiesUpdate {
        actor_unique_id: data.target_player_raw_id,
        player_permission,
        command_permission,
        layers,
    }
}
