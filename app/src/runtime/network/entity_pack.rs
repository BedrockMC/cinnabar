//! Server-pack entities for one session: compiled at admission into their own index space
//! and layered over the vanilla catalog. A bad file skips its entity, never the session.

use std::{collections::HashMap, sync::Arc};

use assets::{ActorArtworkBinding, ActorTexture, RuntimeEntityAssets, RuntimeEquipmentCatalog};
use resource_pack::LayeredPackView;

mod collect;
use collect::collect_files;

use super::resource_packs::{StackFingerprint, parse_pack_json, stack_fingerprint};

/// The pack's entity catalog with the artwork of its eligible rigs.
#[derive(Debug)]
pub(crate) struct SessionEntityPack {
    pub(crate) assets: Arc<RuntimeEntityAssets>,
    pub(crate) textures: Arc<[ActorTexture]>,
    pub(crate) bindings: Arc<[ActorArtworkBinding]>,
    /// The pack's attachable bindings and rasters for held and worn items.
    pub(crate) equipment: Option<Arc<assets::RuntimeEquipmentCatalog>>,
}

/// Vanilla definitions server-pack entities may reference, set once at startup when the
/// sidecar loads.
static VANILLA_REFS: std::sync::OnceLock<assets::VanillaEntityRefs> = std::sync::OnceLock::new();

pub(crate) fn set_vanilla_refs(refs: assets::VanillaEntityRefs) {
    let _ = VANILLA_REFS.set(refs);
}

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
    let files = collect_files(view, VANILLA_REFS.get());
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
    let equipment = if compiled.equipment_bindings.is_empty() {
        None
    } else {
        match RuntimeEquipmentCatalog::from_parts(
            compiled.identity,
            compiled.equipment_bindings,
            compiled.equipment_textures,
        ) {
            Ok(catalog) => Some(Arc::new(catalog)),
            Err(error) => {
                bevy::log::warn!(%error, "server pack attachables were rejected");
                None
            }
        }
    };
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
        equipment,
    }))
}

/// Property defaults of each `entities/*.json` behavior definition the stack carries, keyed by
/// entity type. Only the winning copy of a file is read; a malformed property is skipped.
pub(super) fn pack_property_defaults(
    view: &LayeredPackView,
) -> Vec<(Arc<str>, Vec<client_world::PropertyDefault>)> {
    view.winning_files("entities/", "json")
        .into_iter()
        .filter_map(|(_, bytes)| {
            let root = parse_pack_json(&bytes)?;
            let description = &root["minecraft:entity"]["description"];
            let identifier: Arc<str> = description["identifier"].as_str()?.into();
            let defaults = description["properties"]
                .as_object()?
                .iter()
                .filter_map(|(name, definition)| property_default(name, definition))
                .collect::<Vec<_>>();
            (!defaults.is_empty()).then_some((identifier, defaults))
        })
        .collect()
}

fn property_default(
    name: &str,
    definition: &serde_json::Value,
) -> Option<client_world::PropertyDefault> {
    let values = definition["values"].as_array().map(|values| {
        values
            .iter()
            .filter_map(|value| value.as_str().map(Arc::<str>::from))
            .collect::<Arc<[Arc<str>]>>()
    });
    let default = &definition["default"];
    let number = match definition["type"].as_str()? {
        "bool" => f32::from(u8::from(default.as_bool().unwrap_or(false))),
        "int" | "float" => default.as_f64().unwrap_or(0.0) as f32,
        "enum" => default
            .as_str()
            .and_then(|wanted| {
                values
                    .as_deref()?
                    .iter()
                    .position(|value| value.as_ref() == wanted)
            })
            .map_or(0.0, |index| index as f32),
        _ => return None,
    };
    Some(client_world::PropertyDefault {
        name: name.into(),
        values: definition["type"]
            .as_str()
            .filter(|kind| *kind == "enum")
            .and(values),
        default: number,
    })
}

pub(super) fn entity_identifier(bytes: &[u8]) -> Option<String> {
    parse_pack_json(bytes)?["minecraft:client_entity"]["description"]["identifier"]
        .as_str()
        .map(str::to_owned)
}

#[cfg(test)]
mod tests {
    use super::entity_identifier;

    #[test]
    fn property_defaults_read_bool_number_and_enum_index() {
        use super::property_default;
        let json = |text: &str| serde_json::from_str::<serde_json::Value>(text).unwrap();
        let flag = property_default("a", &json(r#"{"type":"bool","default":true}"#)).unwrap();
        assert_eq!(flag.default, 1.0);
        let level = property_default("b", &json(r#"{"type":"int","default":4}"#)).unwrap();
        assert_eq!(level.default, 4.0);
        let variant = property_default(
            "c",
            &json(r#"{"type":"enum","values":["x","y"],"default":"y"}"#),
        )
        .unwrap();
        assert_eq!((variant.default, variant.values.unwrap().len()), (1.0, 2));
        assert!(property_default("d", &json(r#"{"type":"weird"}"#)).is_none());
    }

    #[test]
    fn identifier_is_read_from_the_client_entity_description() {
        let json = br#"{"minecraft:client_entity":{"description":{"identifier":"a:b"}}}"#;
        assert_eq!(entity_identifier(json).as_deref(), Some("a:b"));
        assert_eq!(entity_identifier(b"{}"), None);
    }
}

/// Env-gated: `CINNABAR_PACKCACHE_DIR` names a directory of cached `<uuid>_<version>.zip`
/// packs; prints which pack entities compile to artwork and why the rest fall back.
#[test]
fn report_local_pack_entities() {
    let Some(dir) = std::env::var_os("CINNABAR_PACKCACHE_DIR") else {
        return;
    };
    let mut zips = std::fs::read_dir(dir)
        .unwrap()
        .filter_map(|entry| Some(entry.ok()?.path()))
        .filter(|path| path.extension().is_some_and(|ext| ext == "zip"))
        .collect::<Vec<_>>();
    zips.sort();
    let refs = std::fs::read("../.local/assets/compiled/vanilla-v1.vanillarefs.json")
        .ok()
        .and_then(|bytes| assets::VanillaEntityRefs::from_json(&bytes));
    eprintln!("vanilla refs loaded: {}", refs.is_some());
    for path in zips {
        let Some(view) = super::local_pack::local_pack_view_at(&path) else {
            continue;
        };
        let files = collect_files(&view, refs.as_ref());
        let entity_files = files
            .iter()
            .filter(|(p, _)| p.starts_with("entity/"))
            .count();
        if entity_files == 0 {
            continue;
        }
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        diagnose_references(&name, &files);
        match asset_compiler::compile_actor_pack(files) {
            Ok(Some(c)) => {
                let mut reasons = std::collections::BTreeMap::<String, Vec<String>>::new();
                for fallback in &c.fallbacks {
                    let rig = &c.entities.rig_bindings[fallback.rig as usize];
                    let id = &c.entities.symbols[rig.entity_symbol as usize].identifier;
                    reasons
                        .entry(fallback.reason.to_string())
                        .or_default()
                        .push(id.to_string());
                }
                let rigs = c.entities.rig_bindings.len();
                eprintln!(
                    "PACK {name}: entity_files={entity_files} rigs={rigs} artwork={} skipped={:?}",
                    c.bindings.len(),
                    c.skipped
                );
                for (reason, ids) in reasons {
                    eprintln!(
                        "  {reason}: {} e.g. {:?}",
                        ids.len(),
                        &ids[..ids.len().min(3)]
                    );
                }
            }
            Ok(None) => eprintln!("PACK {name}: entity_files={entity_files} compiled to nothing"),
            Err(error) => eprintln!("PACK {name}: entity_files={entity_files} ERROR {error}"),
        }
    }
}

/// Counts entity references the pack's own files cannot satisfy (vanilla-defined or missing).
fn diagnose_references(name: &str, files: &[(Box<str>, Vec<u8>)]) {
    use std::collections::{BTreeMap, BTreeSet};
    let json = |bytes: &[u8]| parse_pack_json(bytes);
    let mut defined = BTreeSet::new();
    let paths = files
        .iter()
        .map(|(p, _)| p.as_ref())
        .collect::<BTreeSet<_>>();
    for (path, bytes) in files {
        if path.starts_with("render_controllers/")
            && let Some(root) = json(bytes)
            && let Some(map) = root["render_controllers"].as_object()
        {
            defined.extend(map.keys().cloned());
        }
    }
    let mut missing_controllers = BTreeMap::<String, u32>::new();
    let mut missing_textures = BTreeMap::<String, u32>::new();
    for (path, bytes) in files {
        if !path.starts_with("entity/") {
            continue;
        }
        let Some(root) = json(bytes) else { continue };
        let description = &root["minecraft:client_entity"]["description"];
        for entry in description["render_controllers"]
            .as_array()
            .into_iter()
            .flatten()
        {
            let key = entry
                .as_str()
                .map(str::to_owned)
                .or_else(|| entry.as_object()?.keys().next().cloned());
            if let Some(key) = key
                && !defined.contains(&key)
            {
                *missing_controllers.entry(key).or_default() += 1;
            }
        }
        for texture in description["textures"]
            .as_object()
            .into_iter()
            .flatten()
            .filter_map(|(_, v)| v.as_str())
        {
            let present = [".png", ".tga"]
                .iter()
                .any(|ext| paths.contains(format!("{texture}{ext}").as_str()));
            if !present {
                let dir = texture.rsplit_once('/').map_or("", |(d, _)| d);
                *missing_textures.entry(dir.to_owned()).or_default() += 1;
            }
        }
    }
    eprintln!(
        "  refs {name}: undefined_controllers={missing_controllers:?} textures_not_collected_by_dir={missing_textures:?}"
    );
}
