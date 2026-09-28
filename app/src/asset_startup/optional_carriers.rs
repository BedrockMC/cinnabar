//! Loaders for the optional runtime carriers that sit beside the world carrier.

use super::*;

/// Reads the texture key sidecar beside the world carrier; absence or mismatch
/// only disables vanilla block retexturing from server packs.
pub(super) fn load_material_keys(
    world_asset_path: &Path,
    material_count: usize,
) -> Option<assets::MaterialKeys> {
    let path = world_asset_path.with_extension("matkeys.json");
    let file = File::open(&path).ok()?;
    let mut bytes = Vec::new();
    file.take(assets::MAX_MATERIAL_KEYS_BYTES + 1)
        .read_to_end(&mut bytes)
        .ok()?;
    if bytes.len() as u64 > assets::MAX_MATERIAL_KEYS_BYTES {
        return None;
    }
    let keys = assets::MaterialKeys::from_json(&bytes, material_count);
    if keys.is_none() {
        bevy::log::warn!(
            "material key sidecar does not match the world carrier; rebuild with make assets"
        );
    }
    keys
}

pub(super) fn load_entity_assets(
    world_asset_path: &Path,
) -> Result<LoadedEntityAssets, AssetStartupError> {
    let path = entity_asset_path(world_asset_path);
    let file = File::open(&path).map_err(|source| AssetStartupError::EntityAssetsRead {
        path: path.clone(),
        source,
        rebuild_command: ENTITY_ASSETS_COMPILE_COMMAND,
    })?;
    let length = file
        .metadata()
        .map_err(|source| AssetStartupError::EntityAssetsRead {
            path: path.clone(),
            source,
            rebuild_command: ENTITY_ASSETS_COMPILE_COMMAND,
        })?
        .len();
    if length > MAX_ENTITY_ASSET_BLOB_BYTES {
        return Err(AssetStartupError::EntityAssetsTooLarge {
            path,
            max_bytes: MAX_ENTITY_ASSET_BLOB_BYTES,
            rebuild_command: ENTITY_ASSETS_COMPILE_COMMAND,
        });
    }
    let mut bytes = Vec::with_capacity(length as usize);
    file.take(MAX_ENTITY_ASSET_BLOB_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|source| AssetStartupError::EntityAssetsRead {
            path: path.clone(),
            source,
            rebuild_command: ENTITY_ASSETS_COMPILE_COMMAND,
        })?;
    if bytes.len() as u64 > MAX_ENTITY_ASSET_BLOB_BYTES {
        return Err(AssetStartupError::EntityAssetsTooLarge {
            path,
            max_bytes: MAX_ENTITY_ASSET_BLOB_BYTES,
            rebuild_command: ENTITY_ASSETS_COMPILE_COMMAND,
        });
    }
    let identity = Sha256::digest(&bytes).into();
    let runtime = Arc::new(RuntimeEntityAssets::decode(&bytes).map_err(|source| {
        AssetStartupError::EntityAssetsDecode {
            path: path.clone(),
            source: Box::new(source),
            rebuild_command: ENTITY_ASSETS_COMPILE_COMMAND,
        }
    })?);
    let expected_manifest_sha256 = canonical_source_manifest_sha256(VANILLA_SOURCE_JSON);
    let actual_manifest_sha256 = runtime.source_manifest_sha256();
    if actual_manifest_sha256 != expected_manifest_sha256 {
        return Err(AssetStartupError::EntityAssetsProvenance {
            path,
            expected: format_sha256(expected_manifest_sha256),
            actual: format_sha256(actual_manifest_sha256),
            rebuild_command: ENTITY_ASSETS_COMPILE_COMMAND,
        });
    }
    Ok(LoadedEntityAssets {
        runtime,
        identity,
        selected_path: path,
    })
}

pub(super) fn load_font_assets(
    world_asset_path: &Path,
) -> Result<LoadedFontAssets, AssetStartupError> {
    let local_path = local_font_asset_path(world_asset_path);
    let (path, file, source_manifest, rebuild_command) = match File::open(&local_path) {
        Ok(file) => (
            local_path,
            file,
            VANILLA_SOURCE_JSON,
            LOCAL_FONT_ASSETS_COMPILE_COMMAND,
        ),
        Err(source) if source.kind() == io::ErrorKind::NotFound => {
            let path = font_asset_path(world_asset_path);
            let file = match File::open(&path) {
                Ok(file) => file,
                Err(source) if source.kind() == io::ErrorKind::NotFound => {
                    return diagnostic_font_assets(path);
                }
                Err(source) => {
                    return Err(AssetStartupError::FontAssetsRead {
                        path,
                        source,
                        rebuild_command: FONT_ASSETS_COMPILE_COMMAND,
                    });
                }
            };
            (path, file, UI_FONT_SOURCE_JSON, FONT_ASSETS_COMPILE_COMMAND)
        }
        Err(source) => {
            return Err(AssetStartupError::FontAssetsRead {
                path: local_path,
                source,
                rebuild_command: LOCAL_FONT_ASSETS_COMPILE_COMMAND,
            });
        }
    };
    let length = file
        .metadata()
        .map_err(|source| AssetStartupError::FontAssetsRead {
            path: path.clone(),
            source,
            rebuild_command,
        })?
        .len();
    if length > MAX_FONT_ASSET_BLOB_BYTES {
        return Err(AssetStartupError::FontAssetsTooLarge {
            path,
            max_bytes: MAX_FONT_ASSET_BLOB_BYTES,
            rebuild_command,
        });
    }
    let mut bytes = Vec::with_capacity(length as usize);
    file.take(MAX_FONT_ASSET_BLOB_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|source| AssetStartupError::FontAssetsRead {
            path: path.clone(),
            source,
            rebuild_command,
        })?;
    if bytes.len() as u64 > MAX_FONT_ASSET_BLOB_BYTES {
        return Err(AssetStartupError::FontAssetsTooLarge {
            path,
            max_bytes: MAX_FONT_ASSET_BLOB_BYTES,
            rebuild_command,
        });
    }
    let expected_manifest_sha256 = canonical_source_manifest_sha256(source_manifest);
    let runtime =
        RuntimeFontCatalog::decode(&bytes, expected_manifest_sha256).map_err(|source| {
            AssetStartupError::FontAssetsDecode {
                path: path.clone(),
                source: Box::new(source),
                rebuild_command,
            }
        })?;
    Ok(LoadedFontAssets {
        runtime: Arc::new(runtime),
        selected_path: path,
        diagnostic: false,
    })
}

/// Quotes a path for copy-paste into the platform shell running `make`.
#[cfg(windows)]
pub(crate) fn shell_quote_path(path: &Path) -> String {
    let path = path.to_string_lossy().replace('\\', "/");
    format!("'{}'", path.replace('\'', "''"))
}

#[cfg(not(windows))]
pub(crate) fn shell_quote_path(path: &Path) -> String {
    format!("'{}'", path.to_string_lossy().replace('\'', "'\"'\"'"))
}

pub(super) fn load_atmosphere_assets(
    world_asset_path: &Path,
) -> Result<LoadedAtmosphereAssets, AssetStartupError> {
    let path = atmosphere_asset_path(world_asset_path);
    let file = File::open(&path).map_err(|source| AssetStartupError::AtmosphereRead {
        path: path.clone(),
        source,
        rebuild_command: ATMOSPHERE_COMPILE_COMMAND,
    })?;
    let length = file
        .metadata()
        .map_err(|source| AssetStartupError::AtmosphereRead {
            path: path.clone(),
            source,
            rebuild_command: ATMOSPHERE_COMPILE_COMMAND,
        })?
        .len();
    if length > MAX_ATMOSPHERE_BLOB_BYTES {
        return Err(AssetStartupError::AtmosphereTooLarge {
            path,
            max_bytes: MAX_ATMOSPHERE_BLOB_BYTES,
            rebuild_command: ATMOSPHERE_COMPILE_COMMAND,
        });
    }
    let mut bytes = Vec::with_capacity(length as usize);
    file.take(MAX_ATMOSPHERE_BLOB_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|source| AssetStartupError::AtmosphereRead {
            path: path.clone(),
            source,
            rebuild_command: ATMOSPHERE_COMPILE_COMMAND,
        })?;
    if bytes.len() as u64 > MAX_ATMOSPHERE_BLOB_BYTES {
        return Err(AssetStartupError::AtmosphereTooLarge {
            path,
            max_bytes: MAX_ATMOSPHERE_BLOB_BYTES,
            rebuild_command: ATMOSPHERE_COMPILE_COMMAND,
        });
    }
    let identity = Sha256::digest(&bytes).into();
    let runtime = Arc::new(RuntimeAtmosphereAssets::decode(&bytes).map_err(|source| {
        AssetStartupError::AtmosphereDecode {
            path: path.clone(),
            source: Box::new(source),
            rebuild_command: ATMOSPHERE_COMPILE_COMMAND,
        }
    })?);
    world_provenance::verify_atmosphere_carrier(&path, &runtime)?;
    Ok(LoadedAtmosphereAssets {
        runtime,
        identity,
        selected_path: path,
    })
}
