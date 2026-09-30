//! Terrain texture key index over a compiled carrier's materials, so a session
//! can retexture vanilla blocks from a server pack's `terrain_texture.json`.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

const SCHEMA: u32 = 1;
/// Largest sidecar file the runtime reads.
pub const MAX_MATERIAL_KEYS_BYTES: u64 = 16 * 1024 * 1024;

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Wire {
    schema: u32,
    materials: u32,
    keys: BTreeMap<String, Vec<u32>>,
}

/// Material ids by the terrain texture key they were compiled from.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct MaterialKeys {
    keys: BTreeMap<Box<str>, Box<[u32]>>,
}

impl MaterialKeys {
    /// Groups `(material id, texture key)` pairs by key, ids sorted and unique.
    pub fn from_entries<K: AsRef<str>>(entries: impl IntoIterator<Item = (u32, K)>) -> Self {
        let mut grouped = BTreeMap::<Box<str>, Vec<u32>>::new();
        for (material, key) in entries {
            grouped
                .entry(key.as_ref().into())
                .or_default()
                .push(material);
        }
        let keys = grouped
            .into_iter()
            .map(|(key, mut ids)| {
                ids.sort_unstable();
                ids.dedup();
                (key, ids.into_boxed_slice())
            })
            .collect();
        Self { keys }
    }

    /// Serializes the sidecar for a carrier with `material_count` materials.
    #[must_use]
    pub fn to_json(&self, material_count: u32) -> Vec<u8> {
        let wire = Wire {
            schema: SCHEMA,
            materials: material_count,
            keys: self
                .keys
                .iter()
                .map(|(key, ids)| (key.to_string(), ids.to_vec()))
                .collect(),
        };
        serde_json::to_vec(&wire).expect("material key sidecar serializes")
    }

    /// Parses a sidecar; `None` unless it matches a carrier of `material_count`
    /// materials exactly.
    #[must_use]
    pub fn from_json(bytes: &[u8], material_count: usize) -> Option<Self> {
        let wire: Wire = serde_json::from_slice(bytes).ok()?;
        if wire.schema != SCHEMA || wire.materials as usize != material_count {
            return None;
        }
        let mut keys = BTreeMap::new();
        for (key, ids) in wire.keys {
            if ids.iter().any(|&id| id as usize >= material_count) {
                return None;
            }
            keys.insert(key.into_boxed_str(), ids.into_boxed_slice());
        }
        Some(Self { keys })
    }

    /// Material ids compiled from `key`; empty when the key is unknown.
    #[must_use]
    pub fn materials(&self, key: &str) -> &[u32] {
        self.keys.get(key).map_or(&[], |ids| ids)
    }

    pub fn keys(&self) -> impl Iterator<Item = &str> {
        self.keys.keys().map(AsRef::as_ref)
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.keys.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::MaterialKeys;

    // The sidecar round-trips and refuses a carrier of a different size.
    #[test]
    fn json_round_trips_and_checks_the_material_count() {
        let keys =
            MaterialKeys::from_entries([(3, "stone"), (1, "stone"), (2, "dirt"), (1, "stone")]);
        assert_eq!(keys.materials("stone"), [1, 3]);
        assert_eq!(keys.materials("missing"), [] as [u32; 0]);
        let json = keys.to_json(4);
        assert_eq!(MaterialKeys::from_json(&json, 4), Some(keys));
        assert_eq!(MaterialKeys::from_json(&json, 5), None);
        assert_eq!(MaterialKeys::from_json(&json, 3), None);
    }
}
