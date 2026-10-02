use super::{decode_pack_texture, parse_pack_json};
use assets::{ParticleEffectFile, ParticleTexture, RuntimeParticleAssets};
use resource_pack::LayeredPackView;
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, sync::Arc};

/// Rebuilds particle effects and their referenced textures from the base plus winning layers.
pub(super) fn prepare_particles(
    view: &LayeredPackView,
    base: Option<&RuntimeParticleAssets>,
) -> render::ParticleSystem {
    let mut effects: BTreeMap<Box<str>, ParticleEffectFile> = base
        .into_iter()
        .flat_map(RuntimeParticleAssets::effects)
        .map(|effect| (effect.identifier.clone(), effect.clone()))
        .collect();
    let mut textures: BTreeMap<Box<str>, ParticleTexture> = base
        .into_iter()
        .flat_map(RuntimeParticleAssets::textures)
        .map(|texture| (texture.path.clone(), texture.clone()))
        .collect();
    let mut effect_bytes: usize = effects.values().map(|effect| effect.bytes.len()).sum();
    let mut texture_bytes: usize = textures.values().map(|texture| texture.rgba8.len()).sum();
    for (_, bytes) in super::layered_json(view, "particles/") {
        if bytes.len() > assets::MAX_PARTICLE_EFFECT_BYTES {
            continue;
        }
        let Some(root) = parse_pack_json(&bytes) else {
            continue;
        };
        let Some(identifier) = root["particle_effect"]["description"]["identifier"].as_str() else {
            continue;
        };
        if identifier.is_empty()
            || identifier.len() > assets::MAX_PARTICLE_KEY_BYTES
            || (effects.len() >= assets::MAX_PARTICLE_EFFECTS && !effects.contains_key(identifier))
        {
            continue;
        }
        let next_bytes = effect_bytes
            - effects
                .get(identifier)
                .map_or(0, |effect| effect.bytes.len())
            + bytes.len();
        if next_bytes > assets::MAX_PARTICLE_CARRIER_BYTES {
            continue;
        }
        effect_bytes = next_bytes;
        effects.insert(
            identifier.into(),
            ParticleEffectFile {
                identifier: identifier.into(),
                bytes: Arc::from(bytes),
            },
        );
    }
    for effect in effects.values() {
        let Some(root) = parse_pack_json(&effect.bytes) else {
            continue;
        };
        let Some(path) =
            root["particle_effect"]["description"]["basic_render_parameters"]["texture"].as_str()
        else {
            continue;
        };
        if path.is_empty()
            || path.len() > assets::MAX_PARTICLE_KEY_BYTES
            || (textures.len() >= assets::MAX_PARTICLE_TEXTURES && !textures.contains_key(path))
        {
            continue;
        }
        if let Some(texture) = decode_pack_texture(view, path) {
            let next_bytes = texture_bytes
                - textures.get(path).map_or(0, |texture| texture.rgba8.len())
                + texture.rgba8.len();
            if next_bytes + effect_bytes > assets::MAX_PARTICLE_CARRIER_BYTES {
                continue;
            }
            texture_bytes = next_bytes;
            textures.insert(
                path.into(),
                ParticleTexture {
                    path: path.into(),
                    width: texture.width,
                    height: texture.height,
                    rgba8: Arc::from(texture.rgba8),
                },
            );
        }
    }
    let textures: Vec<_> = textures.into_values().collect();
    let effects: Vec<_> = effects.into_values().collect();
    let identity = base.map_or_else(
        || Sha256::digest(b"Cinnabar optional particle layer").into(),
        RuntimeParticleAssets::source_manifest_sha256,
    );
    match assets::encode_particle_catalog(identity, &textures, &effects)
        .and_then(|bytes| RuntimeParticleAssets::decode(&bytes))
    {
        Ok(assets) => render::ParticleSystem::from_assets(&assets),
        Err(error) => {
            bevy::log::warn!(%error, "optional particle layers ignored");
            base.map_or_else(
                render::ParticleSystem::default,
                render::ParticleSystem::from_assets,
            )
        }
    }
}
