use std::{path::Path, sync::Arc};

use bevy::log::warn;
use client_world::{RideSeat, SeatDefaults};
use serde_json::Value;

const ENTITIES_DIR: &str = "assets/bedrock-samples/v1.26.30.32-preview/full/behavior_pack/entities";

/// Seat layouts of every rideable entity in the local behavior pack, loaded once; absent or
/// unreadable data degrades to no defaults, so riders keep their streamed pose.
pub(super) fn seat_defaults() -> Arc<SeatDefaults> {
    static DEFAULTS: std::sync::OnceLock<Arc<SeatDefaults>> = std::sync::OnceLock::new();
    Arc::clone(DEFAULTS.get_or_init(|| {
        let defaults = crate::install_layout::InstallLayout::discover()
            .ok()
            .and_then(|layout| load(&layout.resource_root.join(ENTITIES_DIR)));
        if defaults.is_none() {
            warn!(
                "behavior pack entities not found; riders without a streamed seat keep their pose"
            );
        }
        Arc::new(defaults.unwrap_or_default())
    }))
}

fn load(directory: &Path) -> Option<SeatDefaults> {
    let mut defaults = SeatDefaults::default();
    for entry in std::fs::read_dir(directory).ok()?.flatten() {
        let Ok(bytes) = std::fs::read(entry.path()) else {
            continue;
        };
        let Some(json) = resource_pack::normalize_jsonc(&bytes) else {
            continue;
        };
        let Ok(document) = serde_json::from_slice::<Value>(&json) else {
            continue;
        };
        if let Some((identifier, seats)) = rideable_seats(&document) {
            defaults.insert(identifier, seats);
        }
    }
    Some(defaults)
}

/// Entity identifier and seats of the first `minecraft:rideable`, base components before groups.
fn rideable_seats(document: &Value) -> Option<(String, Vec<RideSeat>)> {
    let entity = document.get("minecraft:entity")?;
    let identifier = entity
        .pointer("/description/identifier")?
        .as_str()?
        .to_owned();
    let groups = entity
        .get("component_groups")
        .and_then(Value::as_object)
        .into_iter()
        .flat_map(|groups| groups.values());
    let rideable = std::iter::once(entity.get("components")?)
        .chain(groups)
        .find_map(|components| components.get("minecraft:rideable"))?;
    Some((identifier, parse_seats(rideable.get("seats")?)))
}

/// `seats` is one object or a list of them.
fn parse_seats(seats: &Value) -> Vec<RideSeat> {
    let one = |seat: &Value| {
        let position = seat.get("position")?.as_array()?;
        let axis = |index: usize| position.get(index)?.as_f64().map(|value| value as f32);
        Some(RideSeat {
            position: [axis(0)?, axis(1)?, axis(2)?],
            min_riders: seat
                .get("min_rider_count")
                .and_then(Value::as_u64)
                .map_or(0, |count| count as u32),
            max_riders: seat
                .get("max_rider_count")
                .and_then(Value::as_u64)
                .map_or(u32::MAX, |count| count as u32),
        })
    };
    match seats {
        Value::Array(list) => list.iter().filter_map(one).collect(),
        single => one(single).into_iter().collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::rideable_seats;

    #[test]
    fn seats_read_from_components_or_groups_as_object_or_list() {
        let pig = serde_json::json!({"minecraft:entity": {
            "description": {"identifier": "minecraft:pig"},
            "component_groups": {"saddled": {"minecraft:rideable": {"seats": {"position": [0, 0.7, 0]}}}}
        }});
        let (name, seats) = rideable_seats(&pig).unwrap();
        assert_eq!((name.as_str(), seats.len()), ("minecraft:pig", 1));
        assert_eq!(seats[0].position, [0.0, 0.7, 0.0]);
        assert_eq!(seats[0].max_riders, u32::MAX);
        let boat = serde_json::json!({"minecraft:entity": {
            "description": {"identifier": "minecraft:boat"},
            "components": {"minecraft:rideable": {"seats": [
                {"position": [0, -0.2, 0], "min_rider_count": 0, "max_rider_count": 1},
                {"position": [0.2, -0.2, 0], "min_rider_count": 2, "max_rider_count": 2}
            ]}}
        }});
        assert_eq!(rideable_seats(&boat).unwrap().1.len(), 2);
    }
}
