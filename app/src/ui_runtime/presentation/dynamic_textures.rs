//! Composition of the bounded dynamic UI texture pages.

use std::sync::Arc;

use render::UiTexturePage;

use super::{IconRef, UiPresentationRuntime, item_viewmodel, menu_artwork, player_preview};

/// Dynamic page offset holding the session's server item icons.
pub(super) const SESSION_ICON_PAGE: usize = 9;
/// Dynamic pages after the general ten, holding the session's glyph-sheet atlas.
pub(super) const GLYPH_PAGES: usize = 8;
/// Dynamic page offset of the server resource-pack UI textures, after the glyphs.
pub(super) const SERVER_UI_PAGE: usize = 10 + GLYPH_PAGES;
/// Dynamic pages reserved for server resource-pack UI textures.
pub(super) const SERVER_UI_PAGES: usize = render::MAX_UI_DYNAMIC_PAGES - SERVER_UI_PAGE;

pub(super) fn observe_session(runtime: &mut UiPresentationRuntime, session: u64) {
    let changed = runtime
        .texture_session
        .is_some_and(|previous| previous != session);
    runtime.texture_session = Some(session);
    if !changed {
        return;
    }
    runtime.player_preview_source_hash = None;
    runtime.player_preview_pose = None;
    runtime.player_preview_pixels = None;
    runtime.held_viewmodel_source = None;
    runtime.offhand_viewmodel_source = None;
    runtime.menu_artwork_paths.clear();
    runtime.menu_artwork_dirty = true;
    runtime.preview_dirty = true;
    rebuild(runtime);
}

/// Rebuilds dynamic pages from immutable base assets so refreshed launcher
/// artwork cannot accumulate stale layers or discard the HUD carriers.
pub(super) fn rebuild(runtime: &mut UiPresentationRuntime) {
    let width = 256;
    let height = 256;
    let layer_bytes = (width * height * 4) as usize;
    let mut rgba8 = if runtime.preview_dirty {
        vec![0; layer_bytes]
    } else {
        Vec::new()
    };
    let first_dynamic = runtime.textures.dynamic_start();

    if runtime.preview_dirty {
        runtime.player_preview_page = None;
        runtime.player_preview_icon = None;
        runtime.left_hand_icon = None;
        runtime.right_hand_icon = None;
        runtime.held_viewmodel_icon = None;
        runtime.offhand_viewmodel_icon = None;
    }

    let preview_fits = runtime.preview_dirty
        && runtime.player_preview_pixels.is_some()
        && width >= player_preview::PREVIEW_WIDTH
        && height >= player_preview::PREVIEW_HEIGHT
        && width >= player_preview::HAND_WIDTH.saturating_mul(2)
        && height >= player_preview::PREVIEW_HEIGHT.saturating_add(player_preview::HAND_HEIGHT);
    let viewmodel_fits = width >= item_viewmodel::MAIN_ORIGIN[0] + item_viewmodel::SIDE
        && height >= item_viewmodel::OFFHAND_ORIGIN[1] + item_viewmodel::SIDE;
    if preview_fits {
        let page = first_dynamic as u16;
        let layer_start = 0;
        let texture_width = width as usize;
        let copy_raster = |target: &mut [u8],
                           raster: &[u8],
                           origin: [u32; 2],
                           raster_width: u32,
                           raster_height: u32| {
            let raster_width = raster_width as usize;
            let raster_height = raster_height as usize;
            for row in 0..raster_height {
                let source_start = row * raster_width * 4;
                let target_start = layer_start
                    + ((origin[1] as usize + row) * texture_width + origin[0] as usize) * 4;
                target[target_start..target_start + raster_width * 4]
                    .copy_from_slice(&raster[source_start..source_start + raster_width * 4]);
            }
        };
        if let Some(rasters) = runtime.player_preview_pixels.as_ref() {
            for row in 0..player_preview::PREVIEW_HEIGHT as usize {
                let source_start = row * player_preview::PREVIEW_WIDTH as usize * 4;
                let target_start = layer_start + row * texture_width * 4;
                let target_end = target_start + player_preview::PREVIEW_WIDTH as usize * 4;
                rgba8[target_start..target_end].copy_from_slice(
                    &rasters.preview
                        [source_start..source_start + player_preview::PREVIEW_WIDTH as usize * 4],
                );
            }
            copy_raster(
                &mut rgba8,
                &rasters.left_hand,
                [0, player_preview::PREVIEW_HEIGHT],
                player_preview::HAND_WIDTH,
                player_preview::HAND_HEIGHT,
            );
            copy_raster(
                &mut rgba8,
                &rasters.right_hand,
                [player_preview::HAND_WIDTH, player_preview::PREVIEW_HEIGHT],
                player_preview::HAND_WIDTH,
                player_preview::HAND_HEIGHT,
            );
        }
        if viewmodel_fits {
            if let Some(main) = runtime
                .held_viewmodel_source
                .and_then(|icon| item_viewmodel::render(&runtime.textures, icon, false))
            {
                copy_raster(
                    &mut rgba8,
                    &main,
                    item_viewmodel::MAIN_ORIGIN,
                    item_viewmodel::SIDE,
                    item_viewmodel::SIDE,
                );
                runtime.held_viewmodel_icon =
                    Some(item_viewmodel::icon_at(page, item_viewmodel::MAIN_ORIGIN));
            }
            if let Some(offhand) = runtime
                .offhand_viewmodel_source
                .and_then(|icon| item_viewmodel::render(&runtime.textures, icon, true))
            {
                copy_raster(
                    &mut rgba8,
                    &offhand,
                    item_viewmodel::OFFHAND_ORIGIN,
                    item_viewmodel::SIDE,
                    item_viewmodel::SIDE,
                );
                runtime.offhand_viewmodel_icon = Some(item_viewmodel::icon_at(
                    page,
                    item_viewmodel::OFFHAND_ORIGIN,
                ));
            }
        }
        runtime.player_preview_page = Some(page);
        runtime.player_preview_icon = Some(IconRef {
            page,
            uv: [
                0,
                0,
                player_preview::PREVIEW_WIDTH as u16,
                player_preview::PREVIEW_HEIGHT as u16,
            ],
        });
        runtime.left_hand_icon = Some(IconRef {
            page,
            uv: [
                0,
                player_preview::PREVIEW_HEIGHT as u16,
                player_preview::HAND_WIDTH as u16,
                player_preview::PREVIEW_HEIGHT as u16 + player_preview::HAND_HEIGHT as u16,
            ],
        });
        runtime.right_hand_icon = Some(IconRef {
            page,
            uv: [
                player_preview::HAND_WIDTH as u16,
                player_preview::PREVIEW_HEIGHT as u16,
                player_preview::HAND_WIDTH.saturating_mul(2) as u16,
                player_preview::PREVIEW_HEIGHT as u16 + player_preview::HAND_HEIGHT as u16,
            ],
        });
    }

    let preview = if runtime.preview_dirty {
        let Ok(page) = UiTexturePage::owned([width, height], rgba8.into()) else {
            return;
        };
        runtime.preview_dirty = false;
        page
    } else {
        runtime.textures.pages()[first_dynamic].clone()
    };
    let mut dynamic = vec![preview];
    let menu_changed = runtime.menu_artwork_dirty;
    if runtime.menu_artwork_dirty {
        runtime.menu_artwork = menu_artwork::load(
            &runtime.menu_artwork_paths,
            width,
            height,
            (first_dynamic + 1) as u16,
            8,
            layer_bytes * 8,
        );
        runtime.menu_artwork_dirty = false;
    }
    let previous = runtime.textures.pages();
    for offset in 0..8 {
        let page = if !menu_changed {
            previous[first_dynamic + 1 + offset].clone()
        } else if let Some(page) = runtime.menu_artwork.pages.get(offset) {
            page.clone()
        } else {
            runtime.blank_dynamic_page.clone()
        };
        dynamic.push(page);
    }
    dynamic.push(
        runtime
            .session_icons
            .page
            .clone()
            .unwrap_or_else(|| runtime.blank_dynamic_page.clone()),
    );
    let glyph_pages = &runtime.session_glyphs.pages;
    dynamic.extend((0..GLYPH_PAGES).map(|offset| {
        glyph_pages
            .get(offset)
            .cloned()
            .unwrap_or_else(|| runtime.blank_dynamic_page.clone())
    }));
    let server_pages = runtime.server_ui_pages();
    dynamic.extend((0..SERVER_UI_PAGES).map(|offset| {
        server_pages
            .get(offset)
            .cloned()
            .unwrap_or_else(|| runtime.blank_dynamic_page.clone())
    }));
    // Equal per-page identities preserve old immutable payload ownership.
    for (offset, page) in dynamic.iter_mut().enumerate() {
        let old = &previous[first_dynamic + offset];
        if old.identity() == page.identity() {
            *page = old.clone();
        }
    }
    if let Ok(textures) = runtime.textures.replace_dynamic(dynamic) {
        runtime.textures = Arc::new(textures);
        // Pixels live solely in the current catalog, not a second cache owner.
        runtime.menu_artwork.pages.clear();
    }
}
