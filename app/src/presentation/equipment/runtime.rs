//! Builds equipment rig layers (held sprite meshes and worn armor) for a body submission.

use std::{collections::BTreeMap, sync::Arc};

use assets::{
    ArmorSlot, EntityDependencyResolution, EquipmentCategory, RuntimeEntityAssets,
    RuntimeEquipmentCatalog, RuntimeIconCatalog,
};
use bevy::prelude::Resource;
use render::{
    ACTOR_LAYER_BODY, ActorArtworkLocation, ActorArtworkPages, ActorRigGeometry,
    ActorRigRenderInput, ActorRigRoute, ActorRigSubmission, EntityRigId, EquipmentRaster,
    RenderBoneTransform, equipment_rig_id, extruded_sprite_vertices, find_geometry_index,
    geometry_bone_names, item_mesh_rig_id,
};

use super::{
    armor::{DEFAULT_LEATHER_RGB, bone_map, hidden_bone, pack_tint, remap_pose},
    atlas::{Placement, SpriteAtlas},
    display::{
        ItemDisplay, LAYER_BOOTS, LAYER_CHESTPLATE, LAYER_HELMET, LAYER_LEGGINGS, LAYER_MAIN_HAND,
        LAYER_OFF_HAND, attach_to_bone, held_sprite_display,
    },
};

/// Generated item meshes kept resident; further distinct items draw nothing.
const MAX_ITEM_MESHES: usize = 512;

/// One stack an actor wears or holds, reduced to what drawing needs.
#[derive(Clone, Debug)]
pub(crate) struct WornItem {
    pub(crate) identifier: Arc<str>,
    pub(crate) metadata: u32,
    /// Whether the stack routes to a flat compiled sprite.
    pub(crate) sprite: bool,
    pub(crate) dye_rgb: Option<u32>,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct ActorEquipmentInput {
    pub(crate) main: Option<WornItem>,
    pub(crate) off: Option<WornItem>,
    /// Helmet, chestplate, leggings, boots.
    pub(crate) armor: [Option<WornItem>; 4],
}

/// One extra instance plus the artwork page/layer its texture lives on.
pub(crate) struct EquipmentPresentation {
    pub(crate) submission: ActorRigSubmission,
    pub(crate) location: ActorArtworkLocation,
}

/// Which first-person arms the player render controller shows.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct FirstPersonArms {
    pub(crate) right: bool,
    pub(crate) left: bool,
}

const FILLED_MAP: &str = "minecraft:filled_map";
const SHIELD: &str = "minecraft:shield";

impl FirstPersonArms {
    /// The right arm shows for an empty hand or a map; the left for a map in either hand (a shield
    /// in the off hand keeps it hidden). The use-item conditions await the item-use queries.
    pub(crate) fn for_hands(main: Option<&str>, off: Option<&str>) -> Self {
        Self {
            right: main.is_none_or(|main| main == FILLED_MAP),
            left: (main == Some(FILLED_MAP) && off != Some(SHIELD)) || off == Some(FILLED_MAP),
        }
    }
}

struct BodyBones {
    names: Vec<Box<str>>,
    right_item: Option<usize>,
    left_item: Option<usize>,
}

struct ArmorGeometry {
    rig: EntityRigId,
    names: Vec<Box<str>>,
}

#[derive(Resource)]
pub(crate) struct EquipmentRuntime {
    assets: Arc<RuntimeEntityAssets>,
    catalog: Option<Arc<RuntimeEquipmentCatalog>>,
    icons: Arc<RuntimeIconCatalog>,
    placements: Vec<Option<Placement>>,
    atlas_locations: Vec<Option<ActorArtworkLocation>>,
    texture_locations: BTreeMap<Box<str>, ActorArtworkLocation>,
    body_bones: BTreeMap<u32, Option<Arc<BodyBones>>>,
    armor_geometry: BTreeMap<Box<str>, Option<Arc<ArmorGeometry>>>,
    armor_maps: BTreeMap<(u32, Box<str>), Arc<[Option<usize>]>>,
    meshes: BTreeMap<usize, Option<EntityRigId>>,
    pending: Vec<ActorRigGeometry>,
}

impl EquipmentRuntime {
    /// Places the item atlas and attachable textures on new artwork pages. Returns the runtime,
    /// the extended artwork, and the entity-catalog geometries the actor scene must register.
    pub(crate) fn build(
        assets: Arc<RuntimeEntityAssets>,
        catalog: Option<Arc<RuntimeEquipmentCatalog>>,
        icons: Arc<RuntimeIconCatalog>,
        artwork: ActorArtworkPages,
    ) -> (Self, ActorArtworkPages, Vec<u32>) {
        let atlas = SpriteAtlas::pack(icons.sprites());
        let atlas_layers = atlas.layers.len();
        let textures = catalog
            .as_ref()
            .map_or(&[][..], |catalog| catalog.textures());
        let mut rasters = atlas.layers;
        rasters.extend(textures.iter().map(|texture| EquipmentRaster {
            width: texture.width,
            height: texture.height,
            rgba8: Arc::clone(&texture.rgba8),
        }));
        let (artwork, locations) = artwork.with_equipment_rasters(&rasters);
        let texture_locations = textures
            .iter()
            .zip(&locations[atlas_layers..])
            .filter_map(|(texture, location)| Some((texture.identifier.clone(), (*location)?)))
            .collect();
        let mut geometries = catalog
            .iter()
            .flat_map(|catalog| catalog.bindings())
            .filter(|binding| {
                matches!(binding.category, EquipmentCategory::Armor { .. })
                    && binding.geometry.resolution == EntityDependencyResolution::Catalog
            })
            .filter_map(|binding| find_geometry_index(&assets, &binding.geometry.identifier))
            .collect::<Vec<_>>();
        geometries.sort_unstable();
        geometries.dedup();
        let runtime = Self {
            assets,
            catalog,
            icons,
            placements: atlas.placements,
            atlas_locations: locations[..atlas_layers].to_vec(),
            texture_locations,
            body_bones: BTreeMap::new(),
            armor_geometry: BTreeMap::new(),
            armor_maps: BTreeMap::new(),
            meshes: BTreeMap::new(),
            pending: Vec::new(),
        };
        (runtime, artwork, geometries)
    }

    /// Geometries generated since the last call; the actor scene must register them before the
    /// next frame is built.
    pub(crate) fn take_pending_geometries(&mut self) -> Vec<ActorRigGeometry> {
        std::mem::take(&mut self.pending)
    }

    /// Equipment layers for one drawn player body, riding the body's own pose and transform.
    pub(crate) fn layers_for(
        &mut self,
        body: &ActorRigSubmission,
        input: &ActorEquipmentInput,
    ) -> Vec<EquipmentPresentation> {
        let mut layers = Vec::new();
        if !matches!(
            body.route,
            ActorRigRoute::Compiled | ActorRigRoute::StaticFallback
        ) || body.input.identity.layer != ACTOR_LAYER_BODY
        {
            return layers;
        }
        let Some((geometry, bones)) = self.body_bones_for(body.input.rig) else {
            return layers;
        };
        let pose_len = bones.names.len();
        if body.input.previous_bones.len() != pose_len || body.input.current_bones.len() != pose_len
        {
            return layers;
        }
        let display = held_sprite_display();
        if let Some(item) = &input.main {
            self.push_held(
                body,
                item,
                LAYER_MAIN_HAND,
                bones.right_item,
                display,
                &mut layers,
            );
        }
        if let Some(item) = &input.off {
            self.push_held(
                body,
                item,
                LAYER_OFF_HAND,
                bones.left_item,
                display,
                &mut layers,
            );
        }
        let slots = [
            (ArmorSlot::Helmet, LAYER_HELMET),
            (ArmorSlot::Chestplate, LAYER_CHESTPLATE),
            (ArmorSlot::Leggings, LAYER_LEGGINGS),
            (ArmorSlot::Boots, LAYER_BOOTS),
        ];
        for ((slot, layer), item) in slots.into_iter().zip(&input.armor) {
            if let Some(item) = item {
                self.push_armor(body, &bones, geometry, slot, layer, item, &mut layers);
            }
        }
        layers
    }

    /// The body pose with every bone but the visible arms (and their sleeves) zero-scaled, as
    /// vanilla's first-person part visibility hides them. `None` when no arm shows or the pose
    /// does not match the body geometry.
    pub(crate) fn mask_first_person(
        &mut self,
        body: &ActorRigSubmission,
        arms: FirstPersonArms,
    ) -> Option<ActorRigSubmission> {
        if !(arms.right || arms.left) {
            return None;
        }
        let (_, bones) = self.body_bones_for(body.input.rig)?;
        let pose_len = bones.names.len();
        if body.input.previous_bones.len() != pose_len || body.input.current_bones.len() != pose_len
        {
            return None;
        }
        let visible = |name: &str| {
            let is = |wanted: &str| name.eq_ignore_ascii_case(wanted);
            (arms.right && (is("rightArm") || is("rightSleeve")))
                || (arms.left && (is("leftArm") || is("leftSleeve")))
        };
        let mask = |pose: &[RenderBoneTransform]| {
            pose.iter()
                .zip(&bones.names)
                .map(|(bone, name)| if visible(name) { *bone } else { hidden_bone() })
                .collect::<Vec<_>>()
        };
        let mut masked = body.clone();
        masked.input.previous_bones = Arc::from(mask(&body.input.previous_bones));
        masked.input.current_bones = Arc::from(mask(&body.input.current_bones));
        Some(masked)
    }

    fn body_bones_for(&mut self, rig: EntityRigId) -> Option<(u32, Arc<BodyBones>)> {
        let geometry = self
            .assets
            .rig_geometries()
            .get(usize::try_from(rig.0).ok()?)?
            .geometry;
        let entry = self.body_bones.entry(geometry).or_insert_with(|| {
            let names = geometry_bone_names(&self.assets, geometry as usize)?;
            let find = |wanted: &str| {
                names
                    .iter()
                    .position(|name| name.eq_ignore_ascii_case(wanted))
            };
            Some(Arc::new(BodyBones {
                right_item: find("rightItem"),
                left_item: find("leftItem"),
                names,
            }))
        });
        entry.clone().map(|bones| (geometry, bones))
    }

    fn push_held(
        &mut self,
        body: &ActorRigSubmission,
        item: &WornItem,
        layer: u8,
        hand: Option<usize>,
        display: ItemDisplay,
        layers: &mut Vec<EquipmentPresentation>,
    ) {
        let Some(hand) = hand.filter(|_| item.sprite) else {
            return;
        };
        let Some(sprite_index) = self.icons.lookup_index(&item.identifier, item.metadata) else {
            return;
        };
        let Some(placement) = self.placements.get(sprite_index).copied().flatten() else {
            return;
        };
        let Some(location) = self.atlas_locations.get(placement.layer).copied().flatten() else {
            return;
        };
        let Some(mesh) = self.mesh_for(sprite_index, placement) else {
            return;
        };
        let (Some(previous), Some(current)) = (
            body.input.previous_bones.get(hand),
            body.input.current_bones.get(hand),
        ) else {
            return;
        };
        let (Some(previous), Some(current)) = (
            attach_to_bone(*previous, display),
            attach_to_bone(*current, display),
        ) else {
            return;
        };
        layers.push(layer_presentation(
            body,
            layer,
            mesh,
            vec![previous],
            vec![current],
            location,
            0,
        ));
    }

    #[allow(clippy::too_many_arguments)]
    fn push_armor(
        &mut self,
        body: &ActorRigSubmission,
        bones: &BodyBones,
        body_geometry: u32,
        slot: ArmorSlot,
        layer: u8,
        item: &WornItem,
        layers: &mut Vec<EquipmentPresentation>,
    ) {
        let Some(catalog) = self.catalog.clone() else {
            return;
        };
        let Some(binding) = catalog.binding(&item.identifier) else {
            return;
        };
        if binding.category != (EquipmentCategory::Armor { slot }) {
            return;
        }
        let Some(location) = self
            .texture_locations
            .get(binding.texture.identifier.as_ref())
            .copied()
        else {
            return;
        };
        let Some(geometry) = self.armor_geometry_for(&binding.geometry.identifier) else {
            return;
        };
        let map = Arc::clone(
            self.armor_maps
                .entry((body_geometry, binding.geometry.identifier.clone()))
                .or_insert_with(|| bone_map(&geometry.names, &bones.names).into()),
        );
        let tint = if binding.material.contains("leather") {
            pack_tint(item.dye_rgb.unwrap_or(DEFAULT_LEATHER_RGB))
        } else {
            0
        };
        layers.push(layer_presentation(
            body,
            layer,
            geometry.rig,
            remap_pose(&map, &body.input.previous_bones),
            remap_pose(&map, &body.input.current_bones),
            location,
            tint,
        ));
    }

    fn armor_geometry_for(&mut self, identifier: &str) -> Option<Arc<ArmorGeometry>> {
        if let Some(entry) = self.armor_geometry.get(identifier) {
            return entry.clone();
        }
        let entry = find_geometry_index(&self.assets, identifier).and_then(|index| {
            Some(Arc::new(ArmorGeometry {
                rig: equipment_rig_id(index),
                names: geometry_bone_names(&self.assets, index as usize)?,
            }))
        });
        self.armor_geometry.insert(identifier.into(), entry.clone());
        entry
    }

    fn mesh_for(&mut self, sprite_index: usize, placement: Placement) -> Option<EntityRigId> {
        if let Some(entry) = self.meshes.get(&sprite_index) {
            return *entry;
        }
        let entry = self.build_mesh(sprite_index, placement);
        self.meshes.insert(sprite_index, entry);
        entry
    }

    fn build_mesh(&mut self, sprite_index: usize, placement: Placement) -> Option<EntityRigId> {
        if self.meshes.len() >= MAX_ITEM_MESHES {
            return None;
        }
        let sprite = self.icons.sprites().get(sprite_index)?;
        let vertices = extruded_sprite_vertices(
            usize::from(sprite.width),
            usize::from(sprite.height),
            &sprite.rgba8,
            placement.uv_rect(),
        )?;
        let id = item_mesh_rig_id(u32::try_from(self.meshes.len()).ok()?);
        let geometry = ActorRigGeometry::new(id, vertices, vec![[0.0; 3]]).ok()?;
        self.pending.push(geometry);
        Some(id)
    }
}

/// An equipment instance that shares `body`'s identity, transform, and generations.
pub(super) fn layer_presentation(
    body: &ActorRigSubmission,
    layer: u8,
    rig: EntityRigId,
    previous: Vec<RenderBoneTransform>,
    current: Vec<RenderBoneTransform>,
    location: ActorArtworkLocation,
    tint: u32,
) -> EquipmentPresentation {
    let mut identity = body.input.identity;
    identity.layer = layer;
    EquipmentPresentation {
        submission: ActorRigSubmission {
            input: ActorRigRenderInput {
                identity,
                rig,
                previous_bones: Arc::from(previous),
                current_bones: Arc::from(current),
                completed_tick: body.input.completed_tick,
                reset_generation: body.input.reset_generation,
            },
            world_from_actor: body.world_from_actor,
            texture_layer: location.layer(),
            route: ActorRigRoute::Compiled,
            tint,
        },
        location,
    }
}
