//! Fail-closed source closure for the deliberately small neutral pose grammar.
use assets::{AssetError, CompiledEntityAssets, EntityAssetKind};
use serde_json::Value;

fn keys(value: &Value, allowed: &[&str]) -> bool {
    value
        .as_object()
        .is_some_and(|object| object.keys().all(|key| allowed.contains(&key.as_str())))
}
fn numeric(value: &Value) -> bool {
    value.as_f64().is_some_and(f64::is_finite)
        || value.as_array().is_some_and(|v| {
            v.len() == 3 && v.iter().all(|v| v.as_f64().is_some_and(f64::is_finite))
        })
}
fn uniform(value: &Value) -> bool {
    value.is_number()
        || value
            .as_array()
            .is_some_and(|v| v.len() == 3 && v[0] == v[1] && v[1] == v[2])
}
fn clip(value: &Value) -> bool {
    if !keys(value, &["bones", "animation_length", "loop"])
        || value.get("loop") != Some(&Value::Bool(true))
    {
        return false;
    }
    value
        .get("bones")
        .and_then(Value::as_object)
        .is_some_and(|bones| {
            bones.values().all(|bone| {
                keys(bone, &["position", "rotation", "scale"])
                    && bone.as_object().unwrap().iter().all(|(property, channel)| {
                        let vector = |value: &Value| {
                            numeric(value) && (property != "scale" || uniform(value))
                        };
                        vector(channel)
                            || channel.as_object().is_some_and(|timeline| {
                                timeline.iter().all(|(time, value)| {
                                    time.parse::<f32>().is_ok_and(|t| t.is_finite() && t >= 0.0)
                                        && vector(value)
                                })
                            })
                    })
            })
        })
}

pub(super) fn supported(
    entities: &CompiledEntityAssets,
    rig_index: usize,
    entity: &Value,
    mut read: impl FnMut(u32) -> Result<Value, AssetError>,
) -> Result<bool, AssetError> {
    let description = &entity["minecraft:client_entity"]["description"];
    let rig = &entities.rig_bindings[rig_index];
    let selected = entities.symbols[rig.render_controller as usize]
        .identifier
        .as_ref();
    if description
        .get("render_controllers")
        .and_then(Value::as_array)
        .is_none_or(|entries| entries.len() != 1 || entries[0].as_str() != Some(selected))
    {
        return Ok(false);
    }
    let Some(roots) = crate::entity::authored_roots(entity) else {
        return Ok(false);
    };
    if roots.len() > 1 {
        return Ok(false);
    }
    let candidate = entities.rig_geometries[rig.first_geometry as usize];
    if usize::from(candidate.animation_count) + usize::from(candidate.controller_count)
        != roots.len()
    {
        return Ok(false);
    }
    let mut geometry = candidate.geometry as usize;
    // Check all authored ancestors, not only the fields retained by the parent.
    for _ in 0..=entities.geometries.len() {
        let compiled = &entities.geometries[geometry];
        let json = read(compiled.source_index)?;
        let definition = json
            .get("minecraft:geometry")
            .and_then(Value::as_array)
            .and_then(|entries| {
                entries.iter().find(|entry| {
                    entry["description"]["identifier"].as_str()
                        == Some(compiled.identifier.as_ref())
                })
            })
            .or_else(|| json.get(compiled.identifier.as_ref()))
            .or_else(|| {
                compiled.inherits.as_ref().and_then(|parent| {
                    json.get(format!("{}:{}", compiled.identifier, parent.identifier))
                })
            });
        let Some(definition) = definition else {
            return Ok(false);
        };
        let Some(bones) = definition.get("bones").and_then(Value::as_array) else {
            return Ok(false);
        };
        if bones.iter().any(|bone| {
            bone.get("binding").is_some()
                || bone.get("bind_pose_rotation").is_some()
                || bone.get("texture_meshes").is_some()
                || bone.get("reset").is_some_and(|v| v != &Value::Bool(false))
                || bone.get("inflate").is_some_and(|v| v.as_f64() != Some(0.0))
                || bone.get("mirror").is_some_and(|v| v != &Value::Bool(false))
        }) {
            return Ok(false);
        }
        let Some(parent) = &compiled.inherits else {
            break;
        };
        let parents: Vec<_> = entities
            .geometries
            .iter()
            .enumerate()
            .filter(|(_, v)| v.identifier == parent.identifier)
            .collect();
        if parents.len() != 1 {
            return Ok(false);
        }
        geometry = parents[0].0;
    }
    let mut clips = std::collections::BTreeSet::new();
    for binding in &entities.rig_animations[candidate.first_animation as usize
        ..candidate.first_animation as usize + usize::from(candidate.animation_count)]
    {
        clips.insert(binding.clip);
    }
    for binding in &entities.rig_controllers[candidate.first_controller as usize
        ..candidate.first_controller as usize + usize::from(candidate.controller_count)]
    {
        let controller = &entities.controllers[binding.controller as usize];
        let symbol = &entities.symbols[controller.symbol as usize];
        let json = read(symbol.source_index)?;
        let definition = &json["animation_controllers"][symbol.identifier.as_ref()];
        if !keys(definition, &["initial_state", "states"]) || controller.state_count != 1 {
            return Ok(false);
        }
        let Some(states) = definition.get("states").and_then(Value::as_object) else {
            return Ok(false);
        };
        if states.len() != 1 || states.values().any(|state| !keys(state, &["animations"])) {
            return Ok(false);
        }
        for state in &entities.controller_states[controller.first_state as usize
            ..controller.first_state as usize + usize::from(controller.state_count)]
        {
            for animation in &entities.controller_animations[state.first_animation as usize
                ..state.first_animation as usize + usize::from(state.animation_count)]
            {
                clips.insert(animation.clip);
            }
        }
    }
    for index in clips {
        let compiled = &entities.animation_clips[index as usize];
        let symbol = &entities.symbols[compiled.symbol as usize];
        if symbol.kind != EntityAssetKind::Animation {
            return Ok(false);
        }
        let json = read(compiled.source)?;
        if !clip(&json["animations"][symbol.identifier.as_ref()]) {
            return Ok(false);
        }
    }
    Ok(true)
}
