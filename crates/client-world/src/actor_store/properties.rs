use std::{collections::HashMap, sync::Arc};

use protocol::ActorPropertySyncEvent;
use world::{BlockEntityNbt, NbtValue};

use super::ActorStore;

/// Most entity types and properties per type retained from property syncs.
const MAX_PROPERTY_TYPES: usize = 1_024;
const MAX_PROPERTIES_PER_TYPE: usize = 256;

/// How a property's stored number reads: enums store the index of their value name.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum PropertyKind {
    Number,
    Enum(Arc<[Arc<str>]>),
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct PropertyDefinition {
    pub(crate) name: Arc<str>,
    pub(crate) kind: PropertyKind,
}

/// Property definitions of every entity type, in wire index order.
#[derive(Debug, Default)]
pub(crate) struct PropertyRegistry {
    by_type: HashMap<Arc<str>, Arc<[PropertyDefinition]>>,
}

impl PropertyRegistry {
    /// Definitions of the entity type an actor is, when the server has synced them.
    pub(crate) fn for_kind(&self, kind: &protocol::ActorKind) -> Option<Arc<[PropertyDefinition]>> {
        let name = match kind {
            protocol::ActorKind::Entity { identifier } => identifier.as_ref(),
            protocol::ActorKind::Player { .. } => "minecraft:player",
        };
        self.get(name).cloned()
    }

    pub(crate) fn get(&self, entity_type: &str) -> Option<&Arc<[PropertyDefinition]>> {
        self.by_type.get(entity_type)
    }

    /// Retains one type's definitions; malformed or over-budget syncs are dropped.
    pub(crate) fn apply(&mut self, event: &ActorPropertySyncEvent) -> bool {
        let Some((entity_type, definitions)) = parse(&event.data) else {
            return false;
        };
        if self.by_type.len() >= MAX_PROPERTY_TYPES && !self.by_type.contains_key(&entity_type) {
            return false;
        }
        self.by_type.insert(entity_type, definitions.into());
        true
    }
}

fn parse(data: &[u8]) -> Option<(Arc<str>, Vec<PropertyDefinition>)> {
    let (nbt, _) = BlockEntityNbt::decode_prefix(data).ok()?;
    let root = nbt.parse()?;
    let entity_type: Arc<str> = Arc::from(root.string("type")?);
    let mut definitions = Vec::new();
    for entry in root
        .list("properties")?
        .iter()
        .take(MAX_PROPERTIES_PER_TYPE)
    {
        let NbtValue::Compound(entry) = entry else {
            // Keep indices aligned with the wire even when one entry is unreadable.
            definitions.push(PropertyDefinition {
                name: Arc::from(""),
                kind: PropertyKind::Number,
            });
            continue;
        };
        let values = entry.list("enum").map(|values| {
            values
                .iter()
                .filter_map(|value| match value {
                    NbtValue::String(name) => Some(Arc::from(name.as_ref())),
                    _ => None,
                })
                .collect::<Arc<[Arc<str>]>>()
        });
        // Wire type 3 is an enum; int, float and bool all read as their stored number.
        let kind = match (entry.integer("type"), values) {
            (Some(3), Some(values)) => PropertyKind::Enum(values),
            _ => PropertyKind::Number,
        };
        definitions.push(PropertyDefinition {
            name: Arc::from(entry.string("name").unwrap_or("")),
            kind,
        });
    }
    Some((entity_type, definitions))
}

impl ActorStore {
    pub(crate) fn apply_property_sync(&mut self, event: &ActorPropertySyncEvent) -> bool {
        self.property_registry.apply(event)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn string(out: &mut Vec<u8>, text: &str) {
        out.push(text.len() as u8);
        out.extend_from_slice(text.as_bytes());
    }

    fn sync(entries: &[(&str, i32, &[&str])]) -> ActorPropertySyncEvent {
        let mut nbt = vec![10, 0, 8];
        string(&mut nbt, "type");
        string(&mut nbt, "minecraft:wolf");
        nbt.extend_from_slice(&[9]);
        string(&mut nbt, "properties");
        nbt.push(10);
        nbt.push((entries.len() as u8) << 1);
        for (name, kind, values) in entries {
            nbt.push(8);
            string(&mut nbt, "name");
            string(&mut nbt, name);
            nbt.push(3);
            string(&mut nbt, "type");
            nbt.push((*kind as u8) << 1);
            if !values.is_empty() {
                nbt.push(9);
                string(&mut nbt, "enum");
                nbt.push(8);
                nbt.push((values.len() as u8) << 1);
                for value in *values {
                    string(&mut nbt, value);
                }
            }
            nbt.push(0);
        }
        nbt.push(0);
        ActorPropertySyncEvent { data: nbt.into() }
    }

    #[test]
    fn registry_keeps_wire_order_and_enum_values() {
        let mut registry = PropertyRegistry::default();
        assert!(registry.apply(&sync(&[
            ("minecraft:angry", 2, &[]),
            ("minecraft:variant", 3, &["pale", "ashen"]),
        ])));
        let definitions = registry.get("minecraft:wolf").unwrap();
        assert_eq!(definitions[0].name.as_ref(), "minecraft:angry");
        assert_eq!(definitions[0].kind, PropertyKind::Number);
        let PropertyKind::Enum(values) = &definitions[1].kind else {
            panic!("enum property");
        };
        assert_eq!(values[1].as_ref(), "ashen");
    }

    #[test]
    fn malformed_sync_is_dropped() {
        let mut registry = PropertyRegistry::default();
        assert!(!registry.apply(&ActorPropertySyncEvent {
            data: Arc::from([10u8, 0, 8].as_slice()),
        }));
    }
}
