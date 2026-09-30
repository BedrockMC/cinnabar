//! Local ability, hunger and equipment facts that gate movement modes.

use protocol::{AbilitiesUpdate, AbilityLayersEvidence};

use super::control_modes::SPRINT_HUNGER_FLOOR;
use crate::ui_runtime::UiRuntime;

const ELYTRA_IDENTIFIER: &str = "minecraft:elytra";
/// Bedrock enchantment ids; provisional until checked against a native item.
const DEPTH_STRIDER_ENCHANTMENT_ID: i16 = 7;
const SOUL_SPEED_ENCHANTMENT_ID: i16 = 36;

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub(super) struct LocalMovementFacts {
    pub ride: Option<super::RideKind>,
    pub ride_seat: Option<[f32; 3]>,
    pub can_fly: bool,
    pub server_flying: bool,
    pub fly_speed: Option<f64>,
    pub vertical_fly_speed: Option<f64>,
    pub creative_flight: bool,
    pub elytra_ready: bool,
    pub depth_strider: u8,
    pub soul_speed: u8,
    pub sprint_blocked: bool,
}

pub(super) fn read(
    ui: Option<&UiRuntime>,
    stream: &client_world::WorldStream,
    use_held: bool,
) -> LocalMovementFacts {
    let Some(ui) = ui else {
        return LocalMovementFacts::default();
    };
    let elytra_ready = ui.gameplay_hud().armor().is_some_and(|slots| {
        !slots.chestplate.is_empty()
            && stream
                .canonical_item_stack(&slots.chestplate)
                .and_then(|stack| stack.identifier)
                .is_some_and(|identifier| &*identifier == ELYTRA_IDENTIFIER)
    });
    let capabilities = ui.game_mode_capabilities();
    let boots_level = |id| {
        ui.gameplay_hud()
            .armor()
            .and_then(|slots| protocol::item_enchantment_level(&slots.boots.extra_data, id))
            .unwrap_or(0)
    };
    let ride = ui.gameplay_hud().mount_unique_id().map(|unique| {
        stream
            .actor_by_unique_id(unique)
            .and_then(|actor| match &actor.kind {
                protocol::ActorKind::Entity { identifier } => {
                    Some(super::RideKind::from_identifier(identifier))
                }
                protocol::ActorKind::Player { .. } => None,
            })
            .unwrap_or(super::RideKind::Other)
    });
    LocalMovementFacts {
        ride,
        ride_seat: ride
            .and(stream.local_rider_seat_pose())
            .map(|(position, _)| position),
        can_fly: capabilities.is_some_and(|capabilities| capabilities.can_fly),
        server_flying: capabilities.is_some_and(|capabilities| capabilities.flying),
        fly_speed: ui
            .local_abilities()
            .and_then(|update| flight_speed(update, |layer| layer.fly_speed_bits)),
        vertical_fly_speed: ui
            .local_abilities()
            .and_then(|update| flight_speed(update, |layer| layer.vertical_fly_speed_bits)),
        creative_flight: capabilities.is_some_and(|capabilities| capabilities.creative_inventory),
        elytra_ready,
        depth_strider: boots_level(DEPTH_STRIDER_ENCHANTMENT_ID),
        soul_speed: boots_level(SOUL_SPEED_ENCHANTMENT_ID),
        sprint_blocked: (ui.survival_stats_visible()
            && ui
                .hud()
                .hunger()
                .is_some_and(|hunger| hunger.current() <= SPRINT_HUNGER_FLOOR))
            || matches!(
                crate::runtime::network::local_item_use(stream, ui, use_held),
                client_world::LocalItemUse::Using
            ),
    }
}

/// The last ability layer's finite positive flight speed for the selected float field.
fn flight_speed(
    update: &AbilitiesUpdate,
    bits: impl Fn(&protocol::AbilityLayerEvidence) -> u32,
) -> Option<f64> {
    let AbilityLayersEvidence::Received(layers) = &update.layers else {
        return None;
    };
    layers
        .iter()
        .rev()
        .map(|layer| f32::from_bits(bits(layer)))
        .find(|speed| speed.is_finite() && *speed > 0.0)
        .map(f64::from)
}

#[cfg(test)]
mod tests {
    use protocol::AbilityLayerEvidence;

    use super::*;

    fn update(speeds: &[f32]) -> AbilitiesUpdate {
        let layers: Vec<AbilityLayerEvidence> = speeds
            .iter()
            .map(|speed| AbilityLayerEvidence {
                layer_type: 1,
                abilities: 0,
                values: 0,
                fly_speed_bits: speed.to_bits(),
                vertical_fly_speed_bits: 0,
                walk_speed_bits: 0,
            })
            .collect();
        AbilitiesUpdate {
            actor_unique_id: 1,
            player_permission: 0,
            command_permission: 0,
            layers: AbilityLayersEvidence::Received(layers.into()),
        }
    }

    #[test]
    fn flight_speed_takes_the_last_usable_layer() {
        assert_eq!(
            flight_speed(&update(&[0.05, 0.1]), |layer| layer.fly_speed_bits),
            Some(f64::from(0.1_f32))
        );
        assert_eq!(
            flight_speed(&update(&[0.05, 0.0]), |layer| layer.fly_speed_bits),
            Some(f64::from(0.05_f32))
        );
        assert_eq!(
            flight_speed(&update(&[f32::NAN, -1.0]), |layer| layer.fly_speed_bits),
            None
        );
        assert_eq!(
            flight_speed(&update(&[]), |layer| layer.fly_speed_bits),
            None
        );
    }
}
