//! Icons for server-defined items, packed onto the last dynamic UI page.

use std::{collections::HashMap, sync::Arc};

use render::UiTexturePage;

use super::{IconRef, UiPresentationRuntime, dynamic_textures};

/// Largest icon side kept as-is; larger sources are reduced to fit.
pub(crate) const MAX_SESSION_ICON_SIDE: u32 = 64;
const PAGE_SIDE: u32 = 256;
const GUTTER: u32 = 1;
const MAX_LOGGED_MISSES: usize = 512;

/// One server item icon in straight-alpha RGBA8.
#[derive(Debug)]
pub(crate) struct SessionIcon {
    pub(crate) identifier: Arc<str>,
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) rgba8: Box<[u8]>,
}

/// The session's server item icons; later entries for an identifier are ignored.
#[derive(Debug, Default)]
pub(crate) struct SessionIcons {
    pub(crate) icons: Vec<SessionIcon>,
    /// Why an item's icon key did not resolve to an image, for diagnostics.
    pub(crate) misses: HashMap<Arc<str>, Box<str>>,
}

/// The packed page and lookups for the icons last seen on the UI runtime.
#[derive(Default)]
pub(super) struct SessionIconPage {
    source: Option<Arc<SessionIcons>>,
    refs: HashMap<Arc<str>, IconRef>,
    pub(super) page: Option<UiTexturePage>,
}

impl UiPresentationRuntime {
    /// Native CrossbowItem::getIcon routes nonzero animation frames to the pulling
    /// atlas. This identity is shared by inventory cells and actual dropped sprites.
    pub(crate) fn item_icon_key<'a>(
        identifier: &'a str,
        metadata: u32,
        charged_projectile: Option<&str>,
        animation_frame: Option<u32>,
    ) -> (&'a str, u32) {
        if identifier != "minecraft:crossbow" {
            return (identifier, metadata);
        }
        let frame = animation_frame.unwrap_or_else(|| {
            crate::item_use::crossbow_animation_frame(None, 0, charged_projectile, false)
        });
        if frame == 0 {
            (identifier, metadata)
        } else {
            ("minecraft:crossbow_pulling", frame - 1)
        }
    }

    /// Resolves an item identity to its icon: a server icon for this session
    /// first, then the vanilla atlas. Unknown items keep only the slot frame.
    pub(crate) fn item_icon(&self, identifier: &str, metadata: u32) -> Option<IconRef> {
        if let Some(icon) = self.session_icons.refs.get(identifier) {
            return Some(*icon);
        }
        let vanilla = self
            .icon_catalog
            .as_ref()
            .and_then(|catalog| catalog.lookup_index(identifier, metadata))
            .and_then(|sprite| self.icon_refs.as_deref()?.get(sprite).copied());
        if vanilla.is_none() {
            self.note_missing_icon(identifier, metadata);
        }
        vanilla
    }

    /// Logs the hotbar's identifiers and icon presence whenever they change.
    pub(crate) fn note_hotbar(&mut self, slots: [Option<(Arc<str>, bool)>; 9]) {
        if slots == self.logged_hotbar {
            return;
        }
        let shown = slots
            .iter()
            .map(|slot| match slot {
                Some((identifier, true)) => identifier.to_string(),
                Some((identifier, false)) => format!("{identifier} (no icon)"),
                None => "-".to_owned(),
            })
            .collect::<Vec<_>>();
        bevy::log::info!(slots = ?shown, "hotbar changed");
        self.logged_hotbar = slots;
    }

    /// Logs once per identifier why no icon resolved: the session, pack and vanilla lookups.
    fn note_missing_icon(&self, identifier: &str, metadata: u32) {
        let Ok(mut seen) = self.missing_icons.lock() else {
            return;
        };
        if seen.len() >= MAX_LOGGED_MISSES || !seen.insert(identifier.to_owned()) {
            return;
        }
        let session = self.session_icons.source.as_ref().map_or_else(
            || "no session icons (no server pack icons)".to_owned(),
            |icons| {
                icons.misses.get(identifier).map_or_else(
                    || "not among the pack stack's item icon keys".to_owned(),
                    |reason| reason.to_string(),
                )
            },
        );
        let vanilla = if self.icon_catalog.is_none() {
            "vanilla icon carrier not loaded"
        } else {
            "not in the vanilla icon catalog"
        };
        bevy::log::info!(
            identifier,
            metadata,
            custom = !identifier.starts_with("minecraft:"),
            "no icon for item: session/pack: {session}; vanilla: {vanilla}"
        );
    }
}

/// Repacks the page when the runtime's icon set changes identity.
pub(super) fn observe(runtime: &mut UiPresentationRuntime, icons: Option<&Arc<SessionIcons>>) {
    let unchanged = match (&runtime.session_icons.source, icons) {
        (Some(current), Some(next)) => Arc::ptr_eq(current, next),
        (None, None) => true,
        _ => false,
    };
    if unchanged {
        return;
    }
    let page_index =
        (runtime.textures.dynamic_start() + dynamic_textures::SESSION_ICON_PAGE) as u16;
    let (page, refs) = icons
        .and_then(|icons| pack(icons, page_index))
        .map_or((None, HashMap::new()), |(page, refs)| (Some(page), refs));
    runtime.session_icons = SessionIconPage {
        source: icons.cloned(),
        refs,
        page,
    };
    dynamic_textures::rebuild(runtime);
}

/// Shelf-packs icons with a replicated gutter; icons that do not fit are left out.
fn pack(
    icons: &SessionIcons,
    page_index: u16,
) -> Option<(UiTexturePage, HashMap<Arc<str>, IconRef>)> {
    let side = PAGE_SIDE as usize;
    let mut rgba8 = vec![0u8; side * side * 4];
    let mut refs = HashMap::new();
    let (mut cursor, mut row_height) = ([0u32; 2], 0u32);
    // Tallest first keeps shelves dense; ties keep input order.
    let mut ordered = icons.icons.iter().collect::<Vec<_>>();
    ordered.sort_by_key(|icon| std::cmp::Reverse(icon.height));
    for icon in ordered {
        let padded = [icon.width + GUTTER * 2, icon.height + GUTTER * 2];
        if refs.contains_key(&icon.identifier)
            || icon.width == 0
            || icon.height == 0
            || icon.width > MAX_SESSION_ICON_SIDE
            || icon.height > MAX_SESSION_ICON_SIDE
            || icon.rgba8.len() != (icon.width * icon.height * 4) as usize
        {
            continue;
        }
        if cursor[0] + padded[0] > PAGE_SIDE {
            cursor = [0, cursor[1] + row_height];
            row_height = 0;
        }
        if cursor[1] + padded[1] > PAGE_SIDE {
            continue;
        }
        for y in 0..padded[1] {
            let source_y = y.saturating_sub(GUTTER).min(icon.height - 1);
            for x in 0..padded[0] {
                let source_x = x.saturating_sub(GUTTER).min(icon.width - 1);
                let source = ((source_y * icon.width + source_x) * 4) as usize;
                let target = (((cursor[1] + y) * PAGE_SIDE + cursor[0] + x) * 4) as usize;
                rgba8[target..target + 4].copy_from_slice(&icon.rgba8[source..source + 4]);
            }
        }
        let [left, top] = [cursor[0] + GUTTER, cursor[1] + GUTTER];
        refs.insert(
            Arc::clone(&icon.identifier),
            IconRef {
                page: page_index,
                uv: [
                    left as u16,
                    top as u16,
                    (left + icon.width) as u16,
                    (top + icon.height) as u16,
                ],
                glint: false,
            },
        );
        cursor[0] += padded[0];
        row_height = row_height.max(padded[1]);
    }
    let page = UiTexturePage::owned([PAGE_SIDE, PAGE_SIDE], rgba8.into()).ok()?;
    Some((page, refs))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stack_icon_identity_retains_loaded_projectile_and_local_frame_override() {
        for projectile in ["minecraft:arrow", "minecraft:firework_rocket"] {
            let frame = crate::item_use::crossbow_animation_frame(None, 0, Some(projectile), false);
            assert_eq!(
                UiPresentationRuntime::item_icon_key(
                    "minecraft:crossbow",
                    73,
                    Some(projectile),
                    None
                ),
                ("minecraft:crossbow_pulling", frame - 1),
            );
            assert_eq!(
                UiPresentationRuntime::item_icon_key(
                    "minecraft:crossbow",
                    73,
                    Some(projectile),
                    Some(0)
                ),
                ("minecraft:crossbow", 73),
            );
        }
        assert_eq!(
            UiPresentationRuntime::item_icon_key("minecraft:crossbow", 73, None, None),
            ("minecraft:crossbow", 73),
        );
        assert_eq!(
            UiPresentationRuntime::item_icon_key(
                "minecraft:stone",
                2,
                Some("minecraft:arrow"),
                Some(1)
            ),
            ("minecraft:stone", 2),
        );
    }
}
