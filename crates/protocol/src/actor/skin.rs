use std::sync::Arc;

use valentine::bedrock::version::v1_26_51::SerializedSkinRef;

use super::{MAX_PLAYER_LIST_SKIN_BYTES, MAX_STANDARD_SKIN_SIDE};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StandardSkin {
    pub width: u32,
    pub height: u32,
    pub rgba8: Arc<[u8]>,
    /// The skin's cape image when it carries a valid one; counts toward the skin byte budget.
    pub cape: Option<CapeImage>,
    /// The skin's own model inputs when it may name a non-default geometry.
    pub geometry: Option<Arc<SkinGeometrySource>>,
}

/// The resource patch and geometry JSON a skin carries; parsed by the actor runtime.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkinGeometrySource {
    pub resource_patch: Arc<str>,
    pub geometry_data: Arc<str>,
}

impl SkinGeometrySource {
    #[must_use]
    pub fn byte_len(&self) -> usize {
        self.resource_patch.len() + self.geometry_data.len()
    }
}

/// Model input bytes one skin may retain; larger models fall back to the default geometry.
pub const MAX_SKIN_GEOMETRY_SOURCE_BYTES: usize = 1024 * 1024;

/// Model inputs are retained only when the skin carries geometry data; without it vanilla draws
/// its classic model.
fn geometry_source(
    skin: &SerializedSkinRef,
    retained_bytes: &mut usize,
) -> Option<Arc<SkinGeometrySource>> {
    let data = skin.geometry_data.trim();
    let bytes = skin.resource_patch.len() + skin.geometry_data.len();
    let next = retained_bytes.checked_add(bytes)?;
    if data.is_empty()
        || data == "null"
        || bytes > MAX_SKIN_GEOMETRY_SOURCE_BYTES
        || next > MAX_PLAYER_LIST_SKIN_BYTES
    {
        return None;
    }
    *retained_bytes = next;
    Some(Arc::new(SkinGeometrySource {
        resource_patch: skin.resource_patch.as_str().into(),
        geometry_data: skin.geometry_data.as_str().into(),
    }))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapeImage {
    pub width: u32,
    pub height: u32,
    pub rgba8: Arc<[u8]>,
}

/// Cape image sizes Bedrock skins use, as `(width, height)`.
const CAPE_DIMENSIONS: [(u32, u32); 4] = [(64, 32), (128, 64), (256, 128), (1024, 512)];

fn normalize_cape(
    image: &valentine::bedrock::version::v1_26_51::SkinImage,
    retained_bytes: &mut usize,
) -> Option<CapeImage> {
    let (width, height) = (image.width, image.height);
    if !CAPE_DIMENSIONS.contains(&(width, height)) {
        return None;
    }
    let expected = usize::try_from(width).ok()? * usize::try_from(height).ok()? * 4;
    let next = retained_bytes.checked_add(expected)?;
    if image.image_bytes.len() != expected || next > MAX_PLAYER_LIST_SKIN_BYTES {
        return None;
    }
    *retained_bytes = next;
    Some(CapeImage {
        width,
        height,
        rgba8: Arc::from(image.image_bytes.as_slice()),
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlayerSkinUnavailable {
    InvalidDimensions,
    InvalidByteLength,
    RetainedBudgetExceeded,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlayerSkin {
    Standard(StandardSkin),
    Unavailable(PlayerSkinUnavailable),
}

pub(super) fn normalize_player_skin(
    skin: SerializedSkinRef,
    retained_bytes: &mut usize,
) -> PlayerSkin {
    // Vanilla rebuilds persona skins from piece assets this client lacks; the sender's baked
    // `image_data` stands in for that rebuild (a provisional approximation).
    let (width, height) = (skin.image_data.width, skin.image_data.height);
    // Legacy 64x32 skins are kept; the renderer expands them to the square layout.
    let legacy = (width, height) == (64, 32);
    if !legacy && (width != height || !matches!(width, 64 | 128 | MAX_STANDARD_SKIN_SIDE)) {
        return PlayerSkin::Unavailable(PlayerSkinUnavailable::InvalidDimensions);
    }
    let Some(expected_bytes) = usize::try_from(width)
        .ok()
        .and_then(|width| usize::try_from(height).ok().map(|height| (width, height)))
        .and_then(|(width, height)| width.checked_mul(height))
        .and_then(|pixels| pixels.checked_mul(4))
    else {
        return PlayerSkin::Unavailable(PlayerSkinUnavailable::InvalidDimensions);
    };
    if skin.image_data.image_bytes.len() != expected_bytes {
        return PlayerSkin::Unavailable(PlayerSkinUnavailable::InvalidByteLength);
    }
    let Some(next_bytes) = retained_bytes.checked_add(expected_bytes) else {
        return PlayerSkin::Unavailable(PlayerSkinUnavailable::RetainedBudgetExceeded);
    };
    if next_bytes > MAX_PLAYER_LIST_SKIN_BYTES {
        return PlayerSkin::Unavailable(PlayerSkinUnavailable::RetainedBudgetExceeded);
    }
    *retained_bytes = next_bytes;
    let cape = normalize_cape(&skin.cape_image_data, retained_bytes);
    let geometry = geometry_source(&skin, retained_bytes);
    PlayerSkin::Standard(StandardSkin {
        width,
        height,
        rgba8: Arc::from(skin.image_data.image_bytes),
        cape,
        geometry,
    })
}
