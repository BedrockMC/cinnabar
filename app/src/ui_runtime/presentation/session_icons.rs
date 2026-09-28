//! Icons for server-defined items, packed onto the last dynamic UI page.

use std::{collections::HashMap, sync::Arc};

use render::UiTexturePage;

use super::{IconRef, UiPresentationRuntime, dynamic_textures};

/// Largest icon side kept as-is; larger sources are reduced to fit.
pub(crate) const MAX_SESSION_ICON_SIDE: u32 = 64;
const PAGE_SIDE: u32 = 256;
const GUTTER: u32 = 1;

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
}

/// The packed page and lookups for the icons last seen on the UI runtime.
#[derive(Default)]
pub(super) struct SessionIconPage {
    source: Option<Arc<SessionIcons>>,
    refs: HashMap<Arc<str>, IconRef>,
    pub(super) page: Option<UiTexturePage>,
}

impl UiPresentationRuntime {
    /// Resolves an item identity to its icon: a server icon for this session
    /// first, then the vanilla atlas. Unknown items keep only the slot frame.
    pub(crate) fn item_icon(&self, identifier: &str, metadata: u32) -> Option<IconRef> {
        if let Some(icon) = self.session_icons.refs.get(identifier) {
            return Some(*icon);
        }
        let sprite = self
            .icon_catalog
            .as_ref()?
            .lookup_index(identifier, metadata)?;
        self.icon_refs.as_deref()?.get(sprite).copied()
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
            },
        );
        cursor[0] += padded[0];
        row_height = row_height.max(padded[1]);
    }
    let page = UiTexturePage::owned([PAGE_SIDE, PAGE_SIDE], rgba8.into()).ok()?;
    Some((page, refs))
}
