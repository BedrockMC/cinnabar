use super::{AssetStartupError, LoadedEntityAssets, shell_quote_path};
use assets::RuntimeActorCatalog;
use sha2::{Digest, Sha256};
use std::{
    fs::File,
    io::Read,
    path::{Path, PathBuf},
};

pub const ACTOR_ASSETS_FILENAME: &str = "vanilla-v1.mcbeact";
pub fn actor_asset_path(world: &Path) -> PathBuf {
    world.with_file_name(ACTOR_ASSETS_FILENAME)
}

pub fn require_actor_assets(
    world: &Path,
    entities: &LoadedEntityAssets,
) -> Result<RuntimeActorCatalog, AssetStartupError> {
    read_coherent_actor_assets(world, entities.selected_path(), entities.identity)
}

pub(crate) fn require_actor_artwork(
    world: &Path,
    entities: &LoadedEntityAssets,
) -> Result<render::ActorArtworkPages, AssetStartupError> {
    let catalog = require_actor_assets(world, entities)?;
    let artwork = render::ActorArtworkPages::new(&catalog);
    eprintln!(
        "loaded neutral unlit actor artwork: bindings={}, textures={}, page budget rejections={}, rest pose fallbacks={} (pose_expression_unverified); lighting/tint/overlay parity incomplete",
        catalog.bindings().len(),
        catalog.textures().len(),
        artwork.rejected_bindings(),
        catalog
            .bindings()
            .iter()
            .filter(|binding| binding.pose_mode == assets::ActorPoseMode::RestPose)
            .count()
    );
    Ok(artwork)
}

fn read_coherent_actor_assets(
    world: &Path,
    entity_path: &Path,
    entity_identity: [u8; 32],
) -> Result<RuntimeActorCatalog, AssetStartupError> {
    let path = actor_asset_path(world);
    let command = format!(
        "make actor-assets ACTOR_ASSET_BLOB={} ACTOR_ASSET_REPORT={}",
        shell_quote_path(&path),
        shell_quote_path(&path.with_file_name("actor-assets.json"))
    );
    let error = |detail: String| AssetStartupError::ActorAssets {
        path: path.clone(),
        detail: detail.into(),
        rebuild_command: command.clone(),
    };
    let read = |path: &Path, limit: usize| -> Result<Vec<u8>, AssetStartupError> {
        let file = File::open(path).map_err(|source| error(source.to_string()))?;
        if file
            .metadata()
            .map_err(|source| error(source.to_string()))?
            .len()
            > limit as u64
        {
            return Err(error("carrier exceeds startup byte bound".into()));
        }
        let mut bytes = Vec::new();
        file.take(limit as u64 + 1)
            .read_to_end(&mut bytes)
            .map_err(|source| error(source.to_string()))?;
        if bytes.len() > limit {
            return Err(error("carrier exceeds startup byte bound".into()));
        }
        Ok(bytes)
    };
    let bytes = read(&path, assets::MAX_ACTOR_CARRIER_BYTES)?;
    let entity_bytes = read(entity_path, super::MAX_ENTITY_ASSET_BLOB_BYTES as usize)?;
    if <[u8; 32]>::from(Sha256::digest(&entity_bytes)) != entity_identity {
        return Err(error(
            "entity carrier changed during startup; rebuild the coherent carrier set".into(),
        ));
    }
    RuntimeActorCatalog::decode(&bytes, &entity_bytes).map_err(|source| error(source.to_string()))
}

#[cfg(test)]
mod tests;
