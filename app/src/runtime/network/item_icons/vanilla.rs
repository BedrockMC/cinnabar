//! Optional raster replacements follow source paths recorded by the pinned item carrier.
use super::{MAX_SESSION_ICONS, first_frame, icon};
use crate::ui_runtime::presentation::SessionIcon;
use assets::{ItemVisualDefinitionRoute, RuntimeEntityAssets};
use resource_pack::LayeredPackView;
use std::{
    collections::HashSet,
    sync::{Arc, OnceLock},
};

static PATHS: OnceLock<Vec<(Arc<str>, String)>> = OnceLock::new();

/// Retains source routes once, so menus can apply packs before any item registry arrives.
pub(crate) fn set_vanilla_item_paths(entities: &RuntimeEntityAssets) {
    let paths = entities
        .item_visuals()
        .iter()
        .filter_map(|item| {
            if item.key.metadata != 0 {
                return None;
            }
            let ItemVisualDefinitionRoute::Sprite { texture } = item.route else {
                return None;
            };
            let source = entities.sources().get(texture.source as usize)?;
            Some((
                Arc::from(item.key.identifier.as_ref()),
                source.path.to_string(),
            ))
        })
        .collect();
    let _ = PATHS.set(paths);
}

/// Appends only files actually supplied by a pack; absent overrides keep their base icons.
pub(super) fn append(view: &LayeredPackView, icons: &mut Vec<SessionIcon>) {
    let Some(paths) = PATHS.get() else {
        return;
    };
    let mut present = icons
        .iter()
        .map(|icon| icon.identifier.clone())
        .collect::<HashSet<_>>();
    for (identifier, path) in paths {
        if icons.len() >= MAX_SESSION_ICONS {
            break;
        }
        if present.contains(identifier) {
            continue;
        }
        let stem = path
            .rsplit_once('.')
            .map_or(path.as_str(), |(stem, _)| stem);
        if let Some(texture) = super::super::resource_packs::decode_pack_texture(view, stem) {
            icons.push(icon(identifier.clone(), first_frame(texture)));
            present.insert(identifier.clone());
        }
    }
}
