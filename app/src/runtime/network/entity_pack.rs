//! Server-pack entities for one session: compiled at admission into their own index space
//! and layered over the vanilla catalog. A bad file skips its entity, never the session.

use std::{collections::HashMap, sync::Arc};

use assets::{ActorArtworkBinding, ActorTexture, RuntimeEntityAssets};
use resource_pack::LayeredPackView;

use super::resource_packs::{StackFingerprint, parse_pack_json, stack_fingerprint};

/// The pack's entity catalog with the artwork of its eligible rigs.
#[derive(Debug)]
pub(crate) struct SessionEntityPack {
    pub(crate) assets: Arc<RuntimeEntityAssets>,
    pub(crate) textures: Arc<[ActorTexture]>,
    pub(crate) bindings: Arc<[ActorArtworkBinding]>,
}

const FAMILIES: [(&str, &[&str]); 6] = [
    ("entity/", &["json"]),
    ("models/entity/", &["json"]),
    ("animations/", &["json"]),
    ("animation_controllers/", &["json"]),
    ("render_controllers/", &["json"]),
    ("textures/entity/", &["json", "png", "tga"]),
];

type CachedEntities = (StackFingerprint, Option<Arc<SessionEntityPack>>);

/// The previous session's compile, reused when the same pack stack rejoins.
static ENTITY_CACHE: std::sync::Mutex<Option<CachedEntities>> = std::sync::Mutex::new(None);

/// Compiles the stack's entity files; `None` when it defines no usable entity.
pub(super) fn compile_session_entities(
    stack: &resource_pack::ValidatedPackStack,
    view: &LayeredPackView,
) -> Option<Arc<SessionEntityPack>> {
    let fingerprint = stack_fingerprint(stack);
    let mut cache = ENTITY_CACHE
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    if let Some((cached, pack)) = cache.as_ref()
        && *cached == fingerprint
    {
        return pack.clone();
    }
    let pack = compile(view);
    *cache = Some((fingerprint, pack.clone()));
    pack
}

fn compile(view: &LayeredPackView) -> Option<Arc<SessionEntityPack>> {
    let mut files: Vec<(Box<str>, Vec<u8>)> = Vec::new();
    for (prefix, extensions) in FAMILIES {
        for extension in extensions {
            files.extend(
                view.winning_files(prefix, extension)
                    .into_iter()
                    .map(|(path, bytes)| (path.into_boxed_str(), bytes.into_vec())),
            );
        }
    }
    drop_shadowed_entities(view, &mut files);
    let compiled = match asset_compiler::compile_actor_pack(files) {
        Ok(Some(compiled)) => compiled,
        Ok(None) => return None,
        Err(error) => {
            bevy::log::warn!(%error, "server pack entities were not applied");
            return None;
        }
    };
    let skipped = compiled.skipped;
    if skipped != asset_compiler::EntityPackSkips::default() || !compiled.fallbacks.is_empty() {
        bevy::log::warn!(
            oversized = skipped.oversized,
            unparsable = skipped.unparsable,
            over_budget = skipped.over_budget,
            isolated = skipped.isolated,
            rigs_without_artwork = compiled.fallbacks.len(),
            "server pack entities are incomplete"
        );
    }
    let assets = match RuntimeEntityAssets::from_compiled(compiled.entities) {
        Ok(assets) => Arc::new(assets),
        Err(error) => {
            bevy::log::warn!(%error, "server pack entity catalog was rejected");
            return None;
        }
    };
    Some(Arc::new(SessionEntityPack {
        assets,
        textures: compiled.textures.into(),
        bindings: compiled.bindings.into(),
    }))
}

/// Keeps only the highest layer's `entity/` file for each client entity identifier.
fn drop_shadowed_entities(view: &LayeredPackView, files: &mut Vec<(Box<str>, Vec<u8>)>) {
    let mut owners: HashMap<String, Box<str>> = HashMap::new();
    for layer in view.layers() {
        for path in layer.files_under("entity/") {
            if !path.ends_with(".json") {
                continue;
            }
            let Ok(Some(bytes)) = layer.read_file(path) else {
                continue;
            };
            if let Some(identifier) = entity_identifier(&bytes) {
                owners.insert(identifier, path.into());
            }
        }
    }
    files.retain(|(path, bytes)| {
        if !path.starts_with("entity/") || !path.ends_with(".json") {
            return true;
        }
        entity_identifier(bytes)
            .is_none_or(|identifier| owners.get(&identifier).is_none_or(|owner| owner == path))
    });
}

fn entity_identifier(bytes: &[u8]) -> Option<String> {
    parse_pack_json(bytes)?["minecraft:client_entity"]["description"]["identifier"]
        .as_str()
        .map(str::to_owned)
}

#[cfg(test)]
mod tests {
    use super::entity_identifier;

    #[test]
    fn identifier_is_read_from_the_client_entity_description() {
        let json = br#"{"minecraft:client_entity":{"description":{"identifier":"a:b"}}}"#;
        assert_eq!(entity_identifier(json).as_deref(), Some("a:b"));
        assert_eq!(entity_identifier(b"{}"), None);
    }
}
