//! Immutable startup artwork pages. Pixel decoding and hashing never run per frame.
use super::{EntityRigId, MAX_RENDERED_PLAYERS, STANDARD_SKIN_BYTES};
use assets::RuntimeActorCatalog;
use bevy::prelude::Resource;
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

pub const MAX_ACTOR_TEXTURE_PAGES: usize = 32;
/// Layers per generic entity page, within every backend's array-layer limit.
const MAX_ACTOR_PAGE_LAYERS: usize = 256;
// Cinnabar declared RGBA allocation ceiling, not retail or measured driver memory.
// Driver overhead and internal upload staging are separate, unmeasured costs.
pub const MAX_ACTOR_GPU_PIXEL_BYTES: usize = 48 * 1024 * 1024;

/// Layers per equipment page, within every backend's array-layer limit.
const MAX_EQUIPMENT_PAGE_LAYERS: usize = 256;

/// One equipment raster (item sprite or attachable texture) to place on a generic page.
#[derive(Clone, Debug)]
pub struct EquipmentRaster {
    pub width: u16,
    pub height: u16,
    pub rgba8: Arc<[u8]>,
}

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
    pub fn shared_pixels(&self) -> Arc<[u8]> {
        Arc::clone(&self.rgba8)
    }

    /// This page box-filtered down by the smallest power of two that fits `limit` per side.
    pub(crate) fn fit_within(&self, limit: u32) -> std::borrow::Cow<'_, Self> {
        let longest = u32::from(self.width.max(self.height));
        if longest <= limit || limit == 0 {
            return std::borrow::Cow::Borrowed(self);
        }
        let factor = longest.div_ceil(limit).next_power_of_two() as usize;
        let (width, height) = (usize::from(self.width), usize::from(self.height));
        let (out_width, out_height) = (width.div_ceil(factor), height.div_ceil(factor));
        let mut rgba8 = Vec::with_capacity(out_width * out_height * 4 * self.layers as usize);
        for layer in self.rgba8.chunks_exact(width * height * 4) {
            for y in 0..out_height {
                for x in 0..out_width {
                    let mut sum = [0u32; 4];
                    let mut count = 0;
                    for source_y in y * factor..((y + 1) * factor).min(height) {
                        for source_x in x * factor..((x + 1) * factor).min(width) {
                            let at = (source_y * width + source_x) * 4;
                            for (total, value) in sum.iter_mut().zip(&layer[at..at + 4]) {
                                *total += u32::from(*value);
                            }
                            count += 1;
                        }
                    }
                    rgba8.extend(sum.map(|total| (total / count) as u8));
                }
            }
        }
        std::borrow::Cow::Owned(Self {
            width: out_width as u16,
            height: out_height as u16,
            layers: self.layers,
            rgba8: rgba8.into(),
        })
    }
}

#[derive(Clone, Debug, Default, Resource)]
pub struct ActorArtworkPages {
    pub(crate) identity: [u8; 32],
    pub(crate) entity_identity: [u8; 32],
    pub(crate) pages: Arc<[ActorTexturePage]>,
    routes: Arc<BTreeMap<EntityRigId, ActorArtworkLocation>>,
    /// Location of every catalog texture by entity-catalog source index.
    source_locations: Arc<BTreeMap<u32, ActorArtworkLocation>>,
    /// `(page, layer)` of every catalog texture; any entity rig may draw these variants.
    entity_locations: Arc<BTreeSet<(u8, u32)>>,
    /// `(page, layer)` of every equipment raster; equipment rigs are not entity routes.
    equipment: Arc<BTreeSet<(u8, u32)>>,
    /// Session pack textures by pack-catalog source index, a separate index space.
    pack_source_locations: Arc<BTreeMap<u32, ActorArtworkLocation>>,
    pack_locations: Arc<BTreeSet<(u8, u32)>>,
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
            for indices in indices.chunks(MAX_ACTOR_PAGE_LAYERS) {
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
        let source_locations: BTreeMap<u32, ActorArtworkLocation> = catalog
            .textures()
            .iter()
            .enumerate()
            .filter_map(|(index, texture)| {
                Some((texture.source, locations.get(&(index as u32)).copied()?))
            })
            .collect();
        let entity_locations: BTreeSet<(u8, u32)> = source_locations
            .values()
            .map(|location| (location.page, location.layer))
            .collect();
        Self {
            source_locations: Arc::new(source_locations),
            entity_locations: Arc::new(entity_locations),
            identity: catalog.identity(),
            entity_identity: catalog.entity_identity(),
            pages: pages.into(),
            routes: Arc::new(routes),
            equipment: Arc::new(BTreeSet::new()),
            pack_source_locations: Arc::default(),
            pack_locations: Arc::default(),
            rejected_bindings,
        }
    }

    /// Appends pages for `rasters`, grouped by size, and returns each raster's location (`None`
    /// when its group would exceed the page or byte budget). The identity changes to cover them.
    #[must_use]
    pub fn with_equipment_rasters(
        mut self,
        rasters: &[EquipmentRaster],
    ) -> (Self, Vec<Option<ActorArtworkLocation>>) {
        let mut groups = BTreeMap::<(u16, u16), Vec<usize>>::new();
        for (index, raster) in rasters.iter().enumerate() {
            if raster.width != 0
                && raster.height != 0
                && raster.rgba8.len() == usize::from(raster.width) * usize::from(raster.height) * 4
            {
                groups
                    .entry((raster.width, raster.height))
                    .or_default()
                    .push(index);
            }
        }
        let mut pages = self.pages.to_vec();
        let mut gpu_bytes = pages
            .iter()
            .fold(MAX_RENDERED_PLAYERS * STANDARD_SKIN_BYTES, |total, page| {
                total + page.rgba8.len()
            });
        let mut locations = vec![None; rasters.len()];
        let mut equipment = (*self.equipment).clone();
        let mut hasher = Sha256::new();
        hasher.update(self.identity);
        for ((width, height), indices) in groups {
            let length = indices
                .iter()
                .map(|index| rasters[*index].rgba8.len())
                .sum::<usize>();
            let layers = indices.len();
            if gpu_bytes
                .checked_add(length)
                .is_none_or(|total| !within_page_budget(pages.len() + 1, total))
                || layers > MAX_EQUIPMENT_PAGE_LAYERS
            {
                continue;
            }
            let page = (pages.len() + 1) as u8;
            let mut pixels = Vec::with_capacity(length);
            for (layer, index) in indices.iter().enumerate() {
                pixels.extend_from_slice(&rasters[*index].rgba8);
                locations[*index] = Some(ActorArtworkLocation {
                    page,
                    layer: layer as u32,
                    pose_mode: assets::ActorPoseMode::CompiledLiteral,
                });
                equipment.insert((page, layer as u32));
            }
            hasher.update(width.to_le_bytes());
            hasher.update(height.to_le_bytes());
            hasher.update(&pixels);
            gpu_bytes += length;
            pages.push(ActorTexturePage {
                width,
                height,
                layers: layers as u32,
                rgba8: pixels.into(),
            });
        }
        if pages.len() != self.pages.len() {
            self.identity = hasher.finalize().into();
            self.pages = pages.into();
            self.equipment = Arc::new(equipment);
        }
        (self, locations)
    }
    /// Appends pages for a session pack's artwork and routes its bindings under pack rig ids;
    /// a page over the byte budget is dropped and its bindings counted as rejected. Replaces
    /// any earlier pack's variant table, so call it on the startup pages each session.
    #[must_use]
    pub fn with_pack_artwork(
        mut self,
        textures: &[assets::ActorTexture],
        bindings: &[assets::ActorArtworkBinding],
    ) -> Self {
        let mut groups = BTreeMap::<(u16, u16), Vec<usize>>::new();
        for (index, texture) in textures.iter().enumerate() {
            groups
                .entry((texture.width, texture.height))
                .or_default()
                .push(index);
        }
        let mut pages = self.pages.to_vec();
        let mut gpu_bytes = pages
            .iter()
            .fold(MAX_RENDERED_PLAYERS * STANDARD_SKIN_BYTES, |total, page| {
                total + page.rgba8.len()
            });
        let mut locations = BTreeMap::new();
        let mut hasher = Sha256::new();
        hasher.update(self.identity);
        for ((width, height), indices) in groups {
            for indices in indices.chunks(MAX_ACTOR_PAGE_LAYERS) {
                let length = indices
                    .iter()
                    .map(|index| textures[*index].rgba8.len())
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
                    pixels.extend_from_slice(&textures[*index].rgba8);
                    locations.insert(
                        *index as u32,
                        ActorArtworkLocation {
                            page,
                            layer: layer as u32,
                            pose_mode: assets::ActorPoseMode::CompiledLiteral,
                        },
                    );
                }
                hasher.update(width.to_le_bytes());
                hasher.update(height.to_le_bytes());
                hasher.update(&pixels);
                gpu_bytes += length;
                pages.push(ActorTexturePage {
                    width,
                    height,
                    layers: indices.len() as u32,
                    rgba8: pixels.into(),
                });
            }
        }
        let pack_source_locations: BTreeMap<u32, ActorArtworkLocation> = textures
            .iter()
            .enumerate()
            .filter_map(|(index, texture)| {
                Some((texture.source, locations.get(&(index as u32)).copied()?))
            })
            .collect();
        self.pack_locations = Arc::new(
            pack_source_locations
                .values()
                .map(|location| (location.page, location.layer))
                .collect(),
        );
        self.pack_source_locations = Arc::new(pack_source_locations);
        let mut routes = (*self.routes).clone();
        let mut accepted = 0;
        for binding in bindings {
            let Some(mut location) = locations.get(&binding.texture).copied() else {
                continue;
            };
            location.pose_mode = binding.pose_mode;
            routes.insert(super::pack_rig_id(binding.geometry_candidate), location);
            accepted += 1;
        }
        self.rejected_bindings += bindings.len() - accepted;
        if pages.len() != self.pages.len() {
            self.identity = hasher.finalize().into();
            self.pages = pages.into();
        }
        self.routes = Arc::new(routes);
        self
    }

    pub fn route(&self, rig: EntityRigId) -> Option<ActorArtworkLocation> {
        self.routes.get(&rig).copied()
    }
    /// Where the catalog texture drawn from entity-catalog source `source` lives, for a body of
    /// rig `rig`; `None` when the rig has no artwork or the source was not built.
    pub fn variant_location(&self, rig: EntityRigId, source: u32) -> Option<ActorArtworkLocation> {
        let route = self.route(rig)?;
        let sources = if super::rig::is_pack_rig_id(rig) {
            &self.pack_source_locations
        } else {
            &self.source_locations
        };
        let mut location = sources.get(&source).copied()?;
        location.pose_mode = route.pose_mode;
        Some(location)
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
        if super::rig::is_equipment_rig_id(rig) {
            return self.equipment.contains(&(location.page, location.layer));
        }
        let variants = if super::rig::is_pack_rig_id(rig) {
            &self.pack_locations
        } else {
            &self.entity_locations
        };
        // A controller's own geometry draws any entity texture of its catalog.
        if super::rig::is_layer_geometry_rig_id(rig) {
            return variants.contains(&(location.page, location.layer));
        }
        match self.route(rig) {
            Some(route) => {
                route == location
                    || (route.pose_mode == location.pose_mode
                        && variants.contains(&(location.page, location.layer)))
            }
            None => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn page_budget_reserves_player_capacity_and_checks_exact_boundaries() {
        assert_eq!(MAX_RENDERED_PLAYERS, 128);
        assert_eq!(MAX_RENDERED_PLAYERS * STANDARD_SKIN_BYTES, 2 * 1024 * 1024);
        assert_eq!(assets::MAX_ACTOR_TEXTURES, 2048);
        assert!(within_page_budget(
            MAX_ACTOR_TEXTURE_PAGES - 1,
            MAX_ACTOR_GPU_PIXEL_BYTES
        ));
        assert!(!within_page_budget(
            MAX_ACTOR_TEXTURE_PAGES,
            MAX_ACTOR_GPU_PIXEL_BYTES
        ));
        assert!(!within_page_budget(
            MAX_ACTOR_TEXTURE_PAGES - 1,
            MAX_ACTOR_GPU_PIXEL_BYTES + 1
        ));
    }

    #[test]
    fn fitting_a_page_halves_until_it_fits_and_averages_texels() {
        let page = ActorTexturePage {
            width: 2,
            height: 6,
            layers: 1,
            rgba8: [[0u8, 0, 0, 255], [255, 255, 255, 255]]
                .repeat(6)
                .concat()
                .into(),
        };
        let fitted = page.fit_within(4);
        assert_eq!((fitted.width, fitted.height), (1, 3));
        assert_eq!(&fitted.rgba8[..4], &[127, 127, 127, 255]);
        assert!(matches!(page.fit_within(6), std::borrow::Cow::Borrowed(_)));
    }

    #[test]
    fn pack_artwork_routes_bindings_under_pack_rig_ids() {
        let texture = |side: u16| assets::ActorTexture {
            source: 0,
            width: side,
            height: side,
            pixel_sha256: [1; 32],
            rgba8: vec![7; usize::from(side) * usize::from(side) * 4].into(),
        };
        let binding = |candidate: u32, texture: u32| assets::ActorArtworkBinding {
            rig: 0,
            geometry_candidate: candidate,
            entity_symbol: 0,
            geometry: 0,
            render_controller: 0,
            texture,
            material: "entity".into(),
            pose_mode: assets::ActorPoseMode::CompiledLiteral,
        };
        let pages = ActorArtworkPages::default()
            .with_pack_artwork(&[texture(2)], &[binding(5, 0), binding(6, 9)]);
        assert_eq!(pages.pages().len(), 1);
        let location = pages.route(crate::actor::pack_rig_id(5)).unwrap();
        assert!(pages.valid(crate::actor::pack_rig_id(5), location));
        assert_eq!(pages.route(crate::actor::pack_rig_id(6)), None);
        assert!(!crate::actor::rig::is_equipment_rig_id(
            crate::actor::pack_rig_id(5)
        ));
        assert!(crate::actor::rig::is_pack_rig_id(
            crate::actor::pack_rig_id(5)
        ));
        assert_eq!(pages.rejected_bindings(), 1);
        assert_ne!(pages.identity(), [0; 32]);
    }

    // Pack render layers name pack-catalog sources, never vanilla ones.
    #[test]
    fn pack_rig_variants_resolve_in_the_pack_source_space() {
        let texture = |source: u32, fill: u8| assets::ActorTexture {
            source,
            width: 2,
            height: 2,
            pixel_sha256: [fill; 32],
            rgba8: vec![fill; 16].into(),
        };
        let binding = assets::ActorArtworkBinding {
            rig: 0,
            geometry_candidate: 0,
            entity_symbol: 0,
            geometry: 0,
            render_controller: 0,
            texture: 0,
            material: "entity".into(),
            pose_mode: assets::ActorPoseMode::CompiledLiteral,
        };
        let pages = ActorArtworkPages::default()
            .with_pack_artwork(&[texture(4, 1), texture(9, 2)], &[binding]);
        let rig = crate::actor::pack_rig_id(0);
        let variant = pages.variant_location(rig, 9).unwrap();
        assert_eq!((variant.page(), variant.layer()), (1, 1));
        assert!(pages.valid(rig, variant));
        assert_eq!(pages.variant_location(rig, 5), None);
        assert_eq!(pages.variant_location(EntityRigId(0), 9), None);
    }

    #[test]
    fn equipment_rasters_group_by_size_and_validate_by_location() {
        let raster = |side: u16| EquipmentRaster {
            width: side,
            height: side,
            rgba8: vec![9; usize::from(side) * usize::from(side) * 4].into(),
        };
        let bad = EquipmentRaster {
            width: 2,
            height: 2,
            rgba8: vec![0; 3].into(),
        };
        let (pages, locations) = ActorArtworkPages::default().with_equipment_rasters(&[
            raster(2),
            bad,
            raster(4),
            raster(2),
        ]);
        assert_eq!(pages.pages().len(), 2);
        assert_ne!(pages.identity(), [0; 32]);
        assert_eq!(locations[1], None);
        let (first, second) = (locations[0].unwrap(), locations[3].unwrap());
        assert_eq!((first.page(), second.page()), (1, 1));
        assert_eq!((first.layer(), second.layer()), (0, 1));
        assert_eq!(locations[2].unwrap().page(), 2);
        let equipment_rig = crate::actor::equipment_rig_id(3);
        assert!(pages.valid(equipment_rig, first));
        let unknown = ActorArtworkLocation { layer: 9, ..first };
        assert!(!pages.valid(equipment_rig, unknown));
    }
}
