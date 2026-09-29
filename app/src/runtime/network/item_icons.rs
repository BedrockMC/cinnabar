//! Session icons for server-defined items from the stack's item_texture.json.

use std::sync::Arc;

use resource_pack::LayeredPackView;

use super::resource_packs::{DecodedTexture, decode_pack_texture, texture_key_paths};
use crate::ui_runtime::presentation::{MAX_SESSION_ICON_SIDE, SessionIcon, SessionIcons};

const MAX_SESSION_ICONS: usize = 512;

/// Resolves each `(identifier, icon key)` through the item texture catalog merged across the
/// whole stack, then `textures/items/<key>`; misses are recorded with their reason.
pub(super) fn compile_session_icons(
    view: &LayeredPackView,
    icon_keys: &[(Arc<str>, Arc<str>)],
) -> Option<Arc<SessionIcons>> {
    if icon_keys.is_empty() {
        return None;
    }
    let paths = texture_key_paths(view, "textures/item_texture.json");
    // Explicit icon components outrank short-name guesses when the icon cap bites.
    let short_name = |identifier: &str| {
        identifier
            .rsplit_once(':')
            .map_or(identifier, |(_, name)| name)
            .to_owned()
    };
    let (guessed, explicit): (Vec<_>, Vec<_>) = icon_keys
        .iter()
        .partition(|(identifier, key)| key.as_ref() == short_name(identifier));
    let mut icons = Vec::new();
    let mut misses = std::collections::HashMap::new();
    for (identifier, key) in explicit.into_iter().chain(guessed) {
        if icons.len() >= MAX_SESSION_ICONS {
            break;
        }
        match resolve_key(view, &paths, key) {
            Ok(texture) => icons.push(icon(Arc::clone(identifier), first_frame(texture))),
            Err(reason) => {
                // A short-name guess that misses is the normal vanilla-item case.
                if key.as_ref() != short_name(identifier) && misses.len() < MAX_SESSION_ICONS {
                    misses.insert(Arc::clone(identifier), reason.into_boxed_str());
                }
            }
        }
    }
    (!icons.is_empty() || !misses.is_empty()).then(|| Arc::new(SessionIcons { icons, misses }))
}

/// The image for icon `key`: the catalog's path, else `textures/items/<key>`.
fn resolve_key(
    view: &LayeredPackView,
    paths: &std::collections::HashMap<String, String>,
    key: &str,
) -> Result<DecodedTexture, String> {
    let mut tried = Vec::new();
    if let Some(path) = paths.get(key) {
        if let Some(texture) = decode_pack_texture(view, path) {
            return Ok(texture);
        }
        tried.push(format!("catalog path {path} has no readable image"));
    } else {
        tried.push(format!("key '{key}' not in the merged item_texture.json"));
    }
    let bare = key.rsplit_once(':').map_or(key, |(_, name)| name);
    for path in [
        format!("textures/items/{key}"),
        format!("textures/items/{bare}"),
    ] {
        if let Some(texture) = decode_pack_texture(view, &path) {
            return Ok(texture);
        }
    }
    tried.push(format!("no textures/items/{bare}"));
    Err(tried.join("; "))
}

/// Vertical strips animate in vanilla; the static icon is their first frame.
fn first_frame(texture: DecodedTexture) -> DecodedTexture {
    let side = texture.width;
    if texture.height <= side || !texture.height.is_multiple_of(side) {
        return texture;
    }
    let bytes = (side * side * 4) as usize;
    DecodedTexture {
        width: side,
        height: side,
        rgba8: texture.rgba8[..bytes].into(),
    }
}

/// Keeps icons up to the page limit as-is and reduces larger ones by
/// nearest-neighbour sampling, preserving aspect.
fn icon(identifier: Arc<str>, texture: DecodedTexture) -> SessionIcon {
    let longest = texture.width.max(texture.height);
    if longest <= MAX_SESSION_ICON_SIDE {
        return SessionIcon {
            identifier,
            width: texture.width,
            height: texture.height,
            rgba8: texture.rgba8,
        };
    }
    let scale = |side: u32| (side * MAX_SESSION_ICON_SIDE / longest).max(1);
    let (width, height) = (scale(texture.width), scale(texture.height));
    let mut rgba8 = Vec::with_capacity((width * height * 4) as usize);
    for y in 0..height {
        let source_y = y * texture.height / height;
        for x in 0..width {
            let source_x = x * texture.width / width;
            let offset = ((source_y * texture.width + source_x) * 4) as usize;
            rgba8.extend_from_slice(&texture.rgba8[offset..offset + 4]);
        }
    }
    SessionIcon {
        identifier,
        width,
        height,
        rgba8: rgba8.into(),
    }
}

#[cfg(test)]
mod tests;
