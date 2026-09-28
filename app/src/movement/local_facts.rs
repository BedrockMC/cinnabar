//! Local ability, hunger and equipment facts that gate movement modes.

use protocol::{AbilitiesUpdate, AbilityLayersEvidence};

use super::control_modes::SPRINT_HUNGER_FLOOR;
use crate::ui_runtime::UiRuntime;

const ELYTRA_IDENTIFIER: &str = "minecraft:elytra";

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub(super) struct LocalMovementFacts {
    pub can_fly: bool,
    pub server_flying: bool,
    pub fly_speed: Option<f64>,
    pub elytra_ready: bool,
    pub sprint_blocked: bool,
}

pub(super) fn read(
    ui: Option<&UiRuntime>,
    stream: &client_world::WorldStream,
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
    LocalMovementFacts {
        can_fly: capabilities.is_some_and(|capabilities| capabilities.can_fly),
        server_flying: capabilities.is_some_and(|capabilities| capabilities.flying),
        fly_speed: ui.local_abilities().and_then(flight_speed),
        elytra_ready,
        sprint_blocked: ui.survival_stats_visible()
            && ui
                .hud()
                .hunger()
                .is_some_and(|hunger| hunger.current() <= SPRINT_HUNGER_FLOOR),
    }
}

/// The last ability layer's finite positive flight speed.
fn flight_speed(update: &AbilitiesUpdate) -> Option<f64> {
    let AbilityLayersEvidence::Received(layers) = &update.layers else {
        return None;
    };
    layers
        .iter()
        .rev()
        .map(|layer| f32::from_bits(layer.fly_speed_bits))
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
            flight_speed(&update(&[0.05, 0.1])),
            Some(f64::from(0.1_f32))
        );
        assert_eq!(
            flight_speed(&update(&[0.05, 0.0])),
            Some(f64::from(0.05_f32))
        );
        assert_eq!(flight_speed(&update(&[f32::NAN, -1.0])), None);
        assert_eq!(flight_speed(&update(&[])), None);
    }
}
