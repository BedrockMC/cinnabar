use std::sync::Arc;

use assets::{BlockFace, NetworkIdMode, RuntimeAssets};
use render::TileRequest;

/// The block's side-face texture as a particle tile, keyed by its atlas layer.
pub(super) fn block_tile(
    assets: &RuntimeAssets,
    mode: NetworkIdMode,
    network_id: u32,
) -> Option<TileRequest> {
    if !assets.is_known(mode, network_id) {
        return None;
    }
    let face = assets.resolve(mode, network_id).face(BlockFace::North);
    let texture = assets.material(face.material_id()).texture;
    let page = assets.texture_pages().get(texture.page() as usize)?;
    let mip = page.texture.mips.first()?;
    let layer = texture.layer();
    if layer >= page.texture.layers {
        return None;
    }
    let stride = (mip.size * mip.size * 4) as usize;
    let start = layer as usize * stride;
    let pixels = mip.rgba8.get(start..start + stride)?;
    Some(TileRequest {
        key: (u64::from(texture.page()) << 32) | u64::from(layer),
        size: mip.size,
        pixels: Arc::from(pixels),
    })
}
