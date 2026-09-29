//! Vanilla entity definitions a server pack may reference without shipping them (render
//! controllers, animations, animation controllers, geometry), built beside the entity carrier.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

const SCHEMA: u32 = 1;
/// Largest sidecar file the runtime reads.
pub const MAX_VANILLA_REFS_BYTES: u64 = 64 * 1024 * 1024;

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct VanillaGeometryFile {
    pub path: String,
    pub text: String,
}

/// Named vanilla definitions by identifier; geometry is carried as whole files.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct VanillaEntityRefs {
    schema: u32,
    pub render_controllers: BTreeMap<String, Value>,
    pub animations: BTreeMap<String, Value>,
    pub animation_controllers: BTreeMap<String, Value>,
    /// Geometry identifier to its file's index in `geometry_files`.
    pub geometry_index: BTreeMap<String, u32>,
    /// Parent identifier of each geometry that inherits (`geometry.x:geometry.parent`).
    pub geometry_parent: BTreeMap<String, String>,
    pub geometry_files: Vec<VanillaGeometryFile>,
}

impl VanillaEntityRefs {
    #[must_use]
    pub fn new() -> Self {
        Self {
            schema: SCHEMA,
            ..Self::default()
        }
    }

    #[must_use]
    pub fn to_json(&self) -> Vec<u8> {
        serde_json::to_vec(self).expect("vanilla entity refs serialize")
    }

    /// `None` for a malformed or other-schema sidecar.
    #[must_use]
    pub fn from_json(bytes: &[u8]) -> Option<Self> {
        let refs: Self = serde_json::from_slice(bytes).ok()?;
        (refs.schema == SCHEMA).then_some(refs)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_round_trips_and_rejects_other_schemas() {
        let mut refs = VanillaEntityRefs::new();
        refs.render_controllers
            .insert("controller.render.default".into(), Value::Null);
        assert_eq!(VanillaEntityRefs::from_json(&refs.to_json()), Some(refs));
        assert_eq!(VanillaEntityRefs::from_json(b"{\"schema\":9}"), None);
    }
}
