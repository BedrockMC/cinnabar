//! Where a JSON-UI texture path draws from, in the vanilla lookup's spirit: the
//! server pack first, then the UI carrier, then the item icon atlas already on
//! the UI texture array, and last vanilla images from the local pack or remote
//! URLs, packed on demand into the reserved server pages. Paths match in any
//! case, as resource paths do.

use std::{
    borrow::Cow,
    collections::HashMap,
    path::PathBuf,
    sync::{Mutex, MutexGuard},
};

use assets::RuntimeUiAssets;
use json_ui::{NineSlice, TextureMeta, TextureSource};

use super::super::IconRef;
use super::remote_images::{RemoteImages, is_remote};
use super::server_pack::ServerAtlas;

/// Texture sources a form engine owns across frames.
#[derive(Default)]
pub(super) struct TextureSet {
    /// A lock only because renders borrow the engine shared.
    atlas: Mutex<ServerAtlas>,
    /// Carrier texture keys by lowercase spelling.
    carrier: HashMap<String, String>,
    /// Item icon atlas sprites by lowercase item texture path.
    icons: HashMap<String, IconRef>,
    /// Texture page of carrier atlas page 0.
    pub(super) first_page: u16,
    /// Texture page of the first reserved server page.
    pub(super) server_page: u16,
    vanilla: Option<PathBuf>,
    pub(super) remote: RemoteImages,
}

impl TextureSet {
    pub(super) fn new(assets: &RuntimeUiAssets, first_page: u16) -> Self {
        let pages = super::super::dynamic_textures::SERVER_UI_PAGES;
        Self {
            atlas: Mutex::new(ServerAtlas::new(&[], pages)),
            carrier: assets
                .textures()
                .iter()
                .map(|texture| (texture.path.to_ascii_lowercase(), texture.path.to_string()))
                .collect(),
            first_page,
            ..Self::default()
        }
    }

    pub(super) fn lock(&self) -> MutexGuard<'_, ServerAtlas> {
        self.atlas
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
    }

    pub(super) fn atlas_mut(&mut self) -> &mut ServerAtlas {
        self.atlas
            .get_mut()
            .unwrap_or_else(|poison| poison.into_inner())
    }

    /// The carrier, icon and vanilla lookups without the server atlas's pack or
    /// residency, for laying out a no-pack screen on another thread.
    pub(super) fn detached(&self) -> Self {
        let pages = super::super::dynamic_textures::SERVER_UI_PAGES;
        let atlas = ServerAtlas::new(&[], pages).with_fallbacks(self.vanilla.clone(), None);
        Self {
            atlas: Mutex::new(atlas),
            carrier: self.carrier.clone(),
            icons: self.icons.clone(),
            vanilla: self.vanilla.clone(),
            remote: RemoteImages::default(),
            ..*self
        }
    }

    /// Drawn textures too big for a server page, with their source bytes.
    pub(super) fn oversized(&self) -> Vec<(String, std::sync::Arc<[u8]>)> {
        self.lock().oversized()
    }

    /// Install a server atlas, wired to the vanilla and remote fallbacks.
    pub(super) fn set_atlas(&mut self, atlas: ServerAtlas, server_page: u16) {
        let atlas = atlas.with_fallbacks(self.vanilla.clone(), Some(self.remote.clone()));
        self.atlas = Mutex::new(atlas);
        self.server_page = server_page;
    }

    /// Item icons by texture path and the local vanilla pack; the atlas
    /// restarts so both apply.
    pub(super) fn set_fallbacks(&mut self, icons: HashMap<String, IconRef>, vanilla: PathBuf) {
        self.icons = icons
            .into_iter()
            .map(|(path, icon)| (path.to_ascii_lowercase(), icon))
            .collect();
        self.vanilla = Some(vanilla);
        let atlas = std::mem::take(self.atlas_mut());
        let server_page = self.server_page;
        self.set_atlas(atlas, server_page);
    }
}

/// One phase's view of the sources: the carrier plus the locked atlas.
pub(super) struct Textures<'a> {
    pub(super) assets: &'a RuntimeUiAssets,
    pub(super) set: &'a TextureSet,
    pub(super) atlas: &'a ServerAtlas,
    /// Downloaded menu artwork by local path, drawn ahead of every other source.
    pub(super) images: Option<&'a HashMap<String, IconRef>>,
}

impl Textures<'_> {
    /// `path` as its source spells it, without an image extension.
    pub(super) fn canonical<'p>(&self, path: &'p str) -> Cow<'p, str> {
        let key = texture_key(path);
        if is_remote(key) || self.atlas.meta(key).is_some() || self.assets.texture(key).is_some() {
            return Cow::Borrowed(key);
        }
        let folded = key.to_ascii_lowercase();
        match self
            .atlas
            .folded(&folded)
            .or_else(|| self.set.carrier.get(&folded).map(String::as_str))
        {
            Some(found) => Cow::Owned(found.to_owned()),
            None => Cow::Borrowed(key),
        }
    }

    fn icon(&self, key: &str) -> Option<IconRef> {
        self.set.icons.get(&key.to_ascii_lowercase()).copied()
    }

    fn image(&self, path: &str) -> Option<IconRef> {
        let images = self.images?;
        images
            .get(path)
            .or_else(|| images.get(texture_key(path)))
            .copied()
    }

    /// The drawn paths the server atlas must hold: pack textures, and what
    /// neither the carrier nor the icon atlas already has.
    pub(super) fn atlas_keys<'p>(&self, paths: impl Iterator<Item = &'p str>) -> Vec<String> {
        paths
            .filter(|path| self.image(path).is_none())
            .map(|path| self.canonical(path))
            .filter(|key| {
                self.atlas.meta(key).is_some()
                    || (self.assets.texture(key).is_none() && self.icon(key).is_none())
            })
            .map(Cow::into_owned)
            .collect()
    }

    /// The texture page and pixel rect `path` draws from.
    pub(super) fn sprite(&self, path: &str) -> Option<(u16, [f32; 4])> {
        if let Some(image) = self.image(path) {
            let [u0, v0, u1, v1] = image.uv.map(f32::from);
            return Some((image.page, [u0, v0, u1 - u0, v1 - v0]));
        }
        let key = self.canonical(path);
        if let Some(server) = self.atlas.placement(&key) {
            return Some((
                self.set.server_page.saturating_add(server.page),
                server.rect.map(f32::from),
            ));
        }
        if let Some(placement) = self.assets.texture(&key) {
            return Some((
                self.set.first_page.saturating_add(placement.page),
                [placement.x, placement.y, placement.width, placement.height].map(f32::from),
            ));
        }
        let icon = self.icon(&key)?;
        let [u0, v0, u1, v1] = icon.uv.map(f32::from);
        Some((icon.page, [u0, v0, u1 - u0, v1 - v0]))
    }
}

impl TextureSource for Textures<'_> {
    fn texture(&self, path: &str) -> Option<TextureMeta> {
        if let Some(image) = self.image(path) {
            let [u0, v0, u1, v1] = image.uv.map(f64::from);
            return Some(TextureMeta {
                base_size: [u1 - u0, v1 - v0],
                nineslice: None,
            });
        }
        let key = self.canonical(path);
        let key = key.as_ref();
        if let Some(meta) = self.atlas.meta(key) {
            return Some(meta);
        }
        if let Some(sidecar) = self.assets.sidecar(key) {
            return Some(TextureMeta {
                base_size: sidecar.base_size.map(f64::from),
                nineslice: sidecar.nineslice.map(|inset| NineSlice {
                    left: f64::from(inset.left),
                    top: f64::from(inset.top),
                    right: f64::from(inset.right),
                    bottom: f64::from(inset.bottom),
                }),
            });
        }
        if let Some(placement) = self.assets.texture(key) {
            return Some(TextureMeta {
                base_size: [f64::from(placement.width), f64::from(placement.height)],
                nineslice: None,
            });
        }
        if let Some(icon) = self.icon(key) {
            let [u0, v0, u1, v1] = icon.uv.map(f64::from);
            return Some(TextureMeta {
                base_size: [u1 - u0, v1 - v0],
                nineslice: None,
            });
        }
        self.atlas.fallback_meta(key)
    }
}

/// Ui json sometimes spells a texture with its file extension; sources key it
/// without one. A URL keeps its whole spelling.
pub(super) fn texture_key(path: &str) -> &str {
    if is_remote(path) {
        return path;
    }
    for extension in [".png", ".jpg", ".jpeg", ".tga"] {
        if let Some(stem) = path.strip_suffix(extension) {
            return stem;
        }
    }
    path
}
