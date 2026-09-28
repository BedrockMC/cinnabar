//! Optional JSON-UI carrier (`make ui-assets`). Its absence, a decode failure, or
//! stale provenance logs once and leaves server forms on the programmatic
//! fallback dialog; startup never fails over it.

use std::{
    fs::File,
    io::Read,
    path::{Path, PathBuf},
    sync::Arc,
};

use assets::{MAX_UI_CARRIER_BYTES, RuntimeUiAssets};

pub(crate) const UI_ASSETS_FILENAME: &str = "vanilla-v1.mcbeui";
const UI_ASSETS_COMPILE_COMMAND: &str = "make ui-assets";

/// The carrier path beside the selected world carrier.
pub(crate) fn ui_asset_path(world_asset_path: &Path) -> PathBuf {
    world_asset_path.with_file_name(UI_ASSETS_FILENAME)
}

/// The decoded carrier, or `None` (with a startup notice) when it is unusable.
pub(crate) fn load_optional_ui_assets(world_asset_path: &Path) -> Option<Arc<RuntimeUiAssets>> {
    let path = ui_asset_path(world_asset_path);
    match read_carrier(&path) {
        Ok(assets) => {
            eprintln!(
                "loaded JSON-UI carrier from {} ({} atlas pages, {} textures, {} ui files)",
                path.display(),
                assets.atlas_pages().len(),
                assets.textures().len(),
                assets.ui_files().len()
            );
            Some(Arc::new(assets))
        }
        Err(reason) => {
            eprintln!(
                "JSON-UI carrier unavailable at {} ({reason}); server forms use the fallback dialog. Build it with `{UI_ASSETS_COMPILE_COMMAND}`.",
                path.display()
            );
            None
        }
    }
}

fn read_carrier(path: &Path) -> Result<RuntimeUiAssets, String> {
    let file = File::open(path).map_err(|error| error.to_string())?;
    let mut bytes = Vec::new();
    file.take(MAX_UI_CARRIER_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| error.to_string())?;
    if bytes.len() > MAX_UI_CARRIER_BYTES {
        return Err(format!("exceeds {MAX_UI_CARRIER_BYTES} bytes"));
    }
    let assets = RuntimeUiAssets::decode(&bytes).map_err(|error| error.to_string())?;
    let expected = crate::asset_startup::canonical_source_manifest_sha256(
        crate::asset_startup::vanilla_source_manifest_json(),
    );
    if assets.source_manifest_sha256() != expected {
        return Err("compiled from a different pinned source manifest".to_owned());
    }
    Ok(assets)
}
