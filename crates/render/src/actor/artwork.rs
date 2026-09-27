//! Immutable startup artwork pages. Pixel decoding and hashing never run per frame.
use super::{EntityRigId, MAX_RENDERED_PLAYERS, STANDARD_SKIN_BYTES};
use assets::RuntimeActorCatalog;
use bevy::prelude::Resource;
use std::{collections::BTreeMap, sync::Arc};

pub const MAX_ACTOR_TEXTURE_PAGES: usize = 8;
// Cinnabar declared RGBA allocation ceiling, not retail or measured driver memory.
// Driver overhead and internal upload staging are separate, unmeasured costs.
pub const MAX_ACTOR_GPU_PIXEL_BYTES: usize = 32 * 1024 * 1024;

fn within_page_budget(generic_pages: usize, declared_pixel_bytes: usize) -> bool {
    generic_pages < MAX_ACTOR_TEXTURE_PAGES && declared_pixel_bytes <= MAX_ACTOR_GPU_PIXEL_BYTES
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ActorArtworkLocation {
    pub(crate) page: u8,
    pub(crate) layer: u32,
    pub(crate) pose_mode: assets::ActorPoseMode,
}
impl ActorArtworkLocation {
    pub fn pose_mode(self) -> assets::ActorPoseMode {
        self.pose_mode
    }
    pub fn page(self) -> u8 {
        self.page
    }
    pub fn layer(self) -> u32 {
        self.layer
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ActorTexturePage {
    pub(crate) width: u16,
    pub(crate) height: u16,
    pub(crate) layers: u32,
    pub(crate) rgba8: Arc<[u8]>,
}
impl ActorTexturePage {
    pub fn dimensions(&self) -> (u16, u16) {
        (self.width, self.height)
    }
    pub fn layers(&self) -> u32 {
        self.layers
    }
    pub fn pixels(&self) -> &[u8] {
        &self.rgba8
    }
}

#[derive(Clone, Debug, Default, Resource)]
pub struct ActorArtworkPages {
    pub(crate) identity: [u8; 32],
    pub(crate) entity_identity: [u8; 32],
    pub(crate) pages: Arc<[ActorTexturePage]>,
    routes: Arc<BTreeMap<EntityRigId, ActorArtworkLocation>>,
    rejected_bindings: usize,
}
impl ActorArtworkPages {
    pub fn new(catalog: &RuntimeActorCatalog) -> Self {
        let mut groups = BTreeMap::<(u16, u16), Vec<usize>>::new();
        for (index, texture) in catalog.textures().iter().enumerate() {
            groups
                .entry((texture.width, texture.height))
                .or_default()
                .push(index);
        }
        let mut pages = Vec::new();
        let mut locations = BTreeMap::new();
        // The existing player page retains all 128 layers and its full byte budget.
        let mut gpu_bytes = MAX_RENDERED_PLAYERS * STANDARD_SKIN_BYTES;
        for ((width, height), indices) in groups {
            let length = indices
                .iter()
                .map(|index| catalog.textures()[*index].rgba8.len())
                .sum::<usize>();
            if gpu_bytes
                .checked_add(length)
                .is_none_or(|total| !within_page_budget(pages.len() + 1, total))
            {
                continue;
            }
            let page = (pages.len() + 1) as u8;
            let mut pixels = Vec::with_capacity(length);
            for (layer, index) in indices.iter().enumerate() {
                pixels.extend_from_slice(&catalog.textures()[*index].rgba8);
                locations.insert(
                    *index as u32,
                    ActorArtworkLocation {
                        page,
                        layer: layer as u32,
                        pose_mode: assets::ActorPoseMode::CompiledLiteral,
                    },
                );
            }
            gpu_bytes += length;
            pages.push(ActorTexturePage {
                width,
                height,
                layers: indices.len() as u32,
                rgba8: pixels.into(),
            });
        }
        let routes: BTreeMap<_, _> = catalog
            .bindings()
            .iter()
            .filter_map(|binding| {
                locations
                    .get(&binding.texture)
                    .copied()
                    .map(|mut location| {
                        location.pose_mode = binding.pose_mode;
                        (EntityRigId(binding.geometry_candidate), location)
                    })
            })
            .collect();
        let rejected_bindings = catalog.bindings().len() - routes.len();
        Self {
            identity: catalog.identity(),
            entity_identity: catalog.entity_identity(),
            pages: pages.into(),
            routes: Arc::new(routes),
            rejected_bindings,
        }
    }
    pub fn route(&self, rig: EntityRigId) -> Option<ActorArtworkLocation> {
        self.routes.get(&rig).copied()
    }
    pub fn rejected_bindings(&self) -> usize {
        self.rejected_bindings
    }
    pub fn identity(&self) -> [u8; 32] {
        self.identity
    }
    pub fn pages(&self) -> &[ActorTexturePage] {
        &self.pages
    }
    pub(crate) fn valid(&self, rig: EntityRigId, location: ActorArtworkLocation) -> bool {
        self.route(rig) == Some(location)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn page_budget_reserves_player_capacity_and_checks_exact_boundaries() {
        assert_eq!(MAX_RENDERED_PLAYERS, 128);
        assert_eq!(MAX_RENDERED_PLAYERS * STANDARD_SKIN_BYTES, 2 * 1024 * 1024);
        assert_eq!(assets::MAX_ACTOR_TEXTURES, 128);
        assert!(within_page_budget(7, MAX_ACTOR_GPU_PIXEL_BYTES));
        assert!(!within_page_budget(8, MAX_ACTOR_GPU_PIXEL_BYTES));
        assert!(!within_page_budget(7, MAX_ACTOR_GPU_PIXEL_BYTES + 1));
    }
}
