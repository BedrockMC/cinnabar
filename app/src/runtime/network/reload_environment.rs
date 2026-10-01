//! Cinnabar extension: prepare optional environment layers for a live pack swap.

use super::resource_packs::{decode_pack_texture, parse_pack_json};
use assets::{
    AtmosphereTexture, BiomeVisualProfile, FogDistance, FogDistanceMode, FogMedium, FogProfile,
};
use bevy::prelude::Resource;
use resource_pack::LayeredPackView;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, sync::Arc};

mod biomes;
mod particles;
pub(super) use biomes::apply_biome_overlay;

/// Original carriers retained so removing every optional layer restores their resources.
#[derive(Resource, Clone)]
pub(crate) struct EnvironmentBase {
    atmosphere: render::AtmosphereTextureAssets,
    particles: Option<Arc<assets::RuntimeParticleAssets>>,
}

impl EnvironmentBase {
    /// Retains startup carrier resources before any optional stack is applied.
    pub(crate) fn new(
        atmosphere: render::AtmosphereTextureAssets,
        particles: Option<Arc<assets::RuntimeParticleAssets>>,
    ) -> Self {
        Self {
            atmosphere,
            particles,
        }
    }
}

pub(super) struct PreparedEnvironment {
    pub(super) atmosphere: Option<render::AtmosphereTextureAssets>,
    pub(super) particles: Option<render::ParticleSystem>,
    pub(super) dependencies: super::pack_reload_diff::Dependencies,
}

/// Decodes and builds environment resources entirely on the pack reload worker.
pub(super) fn prepare_environment(
    view: &LayeredPackView,
    base: &EnvironmentBase,
    atmosphere_changed: bool,
    particles_changed: bool,
) -> PreparedEnvironment {
    use super::pack_reload_diff::{Subscriber, compile};
    let stack = view.shared_stack();
    let mut dependencies = Default::default();
    let atmosphere = atmosphere_changed.then(|| {
        compile(Subscriber::Atmosphere, &stack, &mut dependencies, |view| {
            prepare_atmosphere(view, &base.atmosphere)
        })
    });
    let particles = particles_changed.then(|| {
        compile(Subscriber::Particles, &stack, &mut dependencies, |view| {
            particles::prepare_particles(view, base.particles.as_deref())
        })
    });
    PreparedEnvironment {
        atmosphere,
        particles,
        dependencies,
    }
}

/// Overlays only the environment components the renderer already consumes.
fn prepare_atmosphere(
    view: &LayeredPackView,
    base: &render::AtmosphereTextureAssets,
) -> render::AtmosphereTextureAssets {
    let Some(runtime) = base.runtime() else {
        return base.clone();
    };
    let mut textures = Vec::new();
    let mut digest = Sha256::new();
    digest.update(base.identity());
    for texture in runtime.textures() {
        let Some(decoded) = decode_pack_texture(view, &texture.source_path) else {
            continue;
        };
        digest.update(&decoded.rgba8);
        textures.push(AtmosphereTexture {
            width: decoded.width,
            height: decoded.height,
            pixels_sha256: Sha256::digest(&decoded.rgba8).into(),
            rgba8: decoded.rgba8,
            ..texture.clone()
        });
    }
    let mut fogs: BTreeMap<_, _> = runtime
        .fog_profiles()
        .iter()
        .map(|profile| (profile.identifier.clone(), profile.clone()))
        .collect();
    let mut biomes: BTreeMap<_, _> = runtime
        .biome_profiles()
        .iter()
        .map(|profile| (profile.biome_identifier.clone(), profile.clone()))
        .collect();
    for (path, bytes) in environment_files(view) {
        let Some(root) = parse_pack_json(&bytes) else {
            continue;
        };
        digest.update(path.as_bytes());
        digest.update(&bytes);
        if path.starts_with("fogs/") {
            if let Some(profile) = fog_profile(&root)
                && (fogs.len() < assets::MAX_ENVIRONMENT_PROFILES
                    || fogs.contains_key(&profile.identifier))
            {
                fogs.insert(profile.identifier.clone(), profile);
            }
        } else {
            overlay_biome_profile(&root, &mut biomes);
        }
    }
    if textures.is_empty()
        && fogs.values().eq(runtime.fog_profiles())
        && biomes.values().eq(runtime.biome_profiles())
    {
        return base.clone();
    }
    match runtime.with_resource_pack_overrides(
        &textures,
        &biomes.into_values().collect::<Vec<_>>(),
        &fogs.into_values().collect::<Vec<_>>(),
    ) {
        Ok(runtime) => {
            render::AtmosphereTextureAssets::new(Arc::new(runtime), digest.finalize().into())
        }
        Err(error) => {
            bevy::log::warn!(%error, "optional pack atmosphere ignored");
            base.clone()
        }
    }
}

/// Preserves pack priority when definitions in different files reuse an identifier.
fn environment_files(view: &LayeredPackView) -> Vec<(String, Box<[u8]>)> {
    let mut files = layered_json(view, "fogs/");
    files.extend(layered_json(view, "biomes/"));
    files
}

/// Reads bounded JSON layers, allowing the higher pack to replace a named definition.
fn layered_json(view: &LayeredPackView, prefix: &str) -> Vec<(String, Box<[u8]>)> {
    let mut result = Vec::new();
    let mut total = 0;
    for pack in view.layers() {
        for path in pack
            .files_under(prefix)
            .iter()
            .filter(|path| path.ends_with(".json"))
        {
            let Some(bytes) = pack.read_file(path).ok().flatten() else {
                continue;
            };
            total += bytes.len();
            if total > resource_pack::MAX_WINNING_BYTES
                || result.len() >= resource_pack::MAX_WINNING_FILES
            {
                return result;
            }
            result.push(((*path).to_owned(), bytes));
        }
    }
    result
}

/// Parses finite supported fog distances, dropping unfamiliar media independently.
fn fog_profile(root: &Value) -> Option<FogProfile> {
    let settings = &root["minecraft:fog_settings"];
    let identifier = settings["description"]["identifier"].as_str()?;
    if identifier.is_empty() || identifier.len() > assets::MAX_ENVIRONMENT_IDENTIFIER_BYTES {
        return None;
    }
    let mut distances = Vec::new();
    for (medium, source) in settings["distance"].as_object()? {
        let Some(medium) = FogMedium::from_source_name(medium) else {
            continue;
        };
        let Some(mode) = source["render_distance_type"]
            .as_str()
            .and_then(FogDistanceMode::from_source_name)
        else {
            continue;
        };
        let (Some(start), Some(end), Some(rgb8)) = (
            source["fog_start"].as_f64(),
            source["fog_end"].as_f64(),
            parse_rgb(&source["fog_color"]),
        ) else {
            continue;
        };
        let (start, end) = (start as f32, end as f32);
        if !start.is_finite() || !end.is_finite() || start < 0.0 || end < start {
            continue;
        }
        distances.push(FogDistance {
            medium,
            mode,
            start_bits: start.to_bits(),
            end_bits: end.to_bits(),
            rgb8,
        });
    }
    distances.sort_by_key(|distance| distance.medium);
    (!distances.is_empty()).then(|| FogProfile {
        identifier: identifier.into(),
        distances: distances.into_boxed_slice(),
    })
}

/// Updates known client-biome environment components, retaining absent base values.
fn overlay_biome_profile(root: &Value, profiles: &mut BTreeMap<Box<str>, BiomeVisualProfile>) {
    let biome = &root["minecraft:client_biome"];
    let Some(identifier) = biome["description"]["identifier"].as_str() else {
        return;
    };
    let Some(profile) = profiles.get_mut(identifier) else {
        return;
    };
    let components = &biome["components"];
    for (key, field, target) in [
        (
            "minecraft:fog_appearance",
            "fog_identifier",
            &mut profile.fog_identifier,
        ),
        (
            "minecraft:atmosphere_identifier",
            "atmosphere_identifier",
            &mut profile.atmosphere_identifier,
        ),
        (
            "minecraft:lighting_identifier",
            "lighting_identifier",
            &mut profile.lighting_identifier,
        ),
    ] {
        if let Some(value) = components[key][field].as_str().filter(|value| {
            !value.is_empty() && value.len() <= assets::MAX_ENVIRONMENT_IDENTIFIER_BYTES
        }) {
            *target = value.into();
        }
    }
    if let Some(rgb) = parse_rgb(&components["minecraft:sky_color"]["sky_color"]) {
        profile.sky_rgb8 = Some(rgb);
    }
}

/// Accepts Bedrock's RGB hex strings and numeric RGB triples.
fn parse_rgb(value: &Value) -> Option<u32> {
    if let Some(text) = value.as_str() {
        let text = text.strip_prefix('#').unwrap_or(text);
        return (text.len() == 6)
            .then(|| u32::from_str_radix(text, 16).ok())
            .flatten();
    }
    let channels = value.as_array()?;
    if channels.len() != 3 {
        return None;
    }
    let mut rgb = 0;
    for channel in channels {
        let value = channel.as_f64()?;
        if !(0.0..=1.0).contains(&value) {
            return None;
        }
        rgb = (rgb << 8) | (value * 255.0).round() as u32;
    }
    Some(rgb)
}

#[cfg(test)]
mod tests;
