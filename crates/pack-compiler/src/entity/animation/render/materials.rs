use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};

use assets::{AssetError, EntityAssetSource, EntityRenderMaterialState};
use serde_json::{Map, Value};

use super::super::super::SourcePayloads;
use super::super::clip::read_json;

struct Definition {
    parent: Option<Box<str>>,
    fields: Map<String, Value>,
}

pub(super) struct MaterialStates {
    definitions: BTreeMap<Box<str>, Option<Definition>>,
}

impl MaterialStates {
    pub(super) fn load(
        root: &Path,
        payloads: &SourcePayloads,
        sources: &[EntityAssetSource],
    ) -> Result<Self, AssetError> {
        let mut definitions = BTreeMap::new();
        for source in sources.iter().filter(|source| {
            source.path.starts_with("materials/") && source.path.ends_with(".material")
        }) {
            let value = read_json(root, payloads, source)?;
            let Some(entries) = value.get("materials").and_then(Value::as_object) else {
                continue;
            };
            for (declaration, value) in entries {
                let Some(fields) = value.as_object() else {
                    continue;
                };
                let (name, parent) = declaration
                    .split_once(':')
                    .map_or((declaration.as_str(), None), |(name, parent)| {
                        (name, Some(parent.into()))
                    });
                let name = name.strip_prefix('+').unwrap_or(name);
                if name.is_empty() || definitions.len() >= assets::MAX_ENTITY_ASSET_SYMBOLS {
                    continue;
                }
                definitions
                    .entry(name.into())
                    .and_modify(|existing| *existing = None)
                    .or_insert_with(|| {
                        Some(Definition {
                            parent,
                            fields: fields.clone(),
                        })
                    });
            }
        }
        Ok(Self { definitions })
    }

    pub(super) fn resolve(&self, target: &str) -> Option<EntityRenderMaterialState> {
        let target = target.strip_suffix(".skinning").unwrap_or(target);
        let mut name = target;
        let mut seen = BTreeSet::new();
        let mut chain = Vec::new();
        let mut state;
        loop {
            if !seen.insert(name) {
                return None;
            }
            let Some(definition) = self.definitions.get(name) else {
                state = builtin(name)?;
                break;
            };
            let definition = definition.as_ref()?;
            chain.push(&definition.fields);
            match definition.parent.as_deref() {
                Some(parent) => name = parent,
                None => {
                    state = builtin(name).unwrap_or_default();
                    break;
                }
            }
        }
        for fields in chain.into_iter().rev() {
            apply(fields, &mut state)?;
        }
        Some(state)
    }
}

fn builtin(name: &str) -> Option<EntityRenderMaterialState> {
    let mut state = EntityRenderMaterialState::default();
    match name {
        "entity" | "entity_static" => {}
        "entity_nocull" => state.cull = false,
        "entity_alphatest" => {
            state.alpha_test = true;
            state.cull = false;
        }
        "entity_alphatest_one_sided" => state.alpha_test = true,
        "entity_alphablend" => state.blend = true,
        _ => return None,
    }
    Some(state)
}

fn apply(fields: &Map<String, Value>, state: &mut EntityRenderMaterialState) -> Option<()> {
    let replace_states = fields.get("states").is_some_and(|value| !value.is_null());
    let replace_defines = fields.get("defines").is_some_and(|value| !value.is_null());
    if replace_states {
        state.cull = true;
        state.blend = false;
        state.depth_write = true;
    }
    if replace_defines {
        state.alpha_test = false;
    }
    for (key, enabled) in [("states", true), ("+states", true), ("-states", false)] {
        if replace_states != (key == "states") {
            continue;
        }
        let Some(values) = fields.get(key).filter(|value| !value.is_null()) else {
            continue;
        };
        for value in values.as_array()? {
            match value.as_str()? {
                "Blending" => state.blend = enabled,
                "DisableCulling" => state.cull = !enabled,
                "DisableDepthWrite" => state.depth_write = !enabled,
                _ => {}
            }
        }
    }
    for (key, enabled) in [("defines", true), ("+defines", true), ("-defines", false)] {
        if replace_defines != (key == "defines") {
            continue;
        }
        let Some(values) = fields.get(key).filter(|value| !value.is_null()) else {
            continue;
        };
        for value in values.as_array()? {
            if value.as_str()? == "ALPHA_TEST" {
                state.alpha_test = enabled;
            }
        }
    }
    Some(())
}
