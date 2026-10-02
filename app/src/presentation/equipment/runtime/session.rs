//! The session's server items: component facts and pack icons, so custom items draw in the
//! hand and on the body as the vanilla client draws component items.

use super::*;

use crate::runtime::network::entity_pack::SessionItems;

/// Session icons get their own mesh id range above the startup item meshes: one per registry
/// item, so no server item goes without its mesh.
const MAX_SESSION_MESHES: usize = protocol::MAX_ITEM_REGISTRY_ENTRIES;

/// A session's icon sprites packed for the artwork pages, before placement on them.
pub(crate) struct StagedSessionIcons {
    sprites: Vec<IconSprite>,
    by_identifier: BTreeMap<Box<str>, BTreeMap<u32, usize>>,
    atlas: SpriteAtlas,
}

impl StagedSessionIcons {
    /// Packs the pack's item icons; `None` when the session has none.
    pub(crate) fn stage(items: Option<&SessionItems>) -> Option<Self> {
        let icons = items?.icons.as_deref()?;
        let mut sprites = Vec::new();
        let mut by_identifier: BTreeMap<Box<str>, BTreeMap<u32, usize>> = BTreeMap::new();
        for icon in icons.icons.iter().take(MAX_SESSION_MESHES) {
            let (Ok(width), Ok(height)) = (u16::try_from(icon.width), u16::try_from(icon.height))
            else {
                continue;
            };
            if by_identifier
                .get(icon.identifier.as_ref())
                .is_some_and(|variants| variants.contains_key(&icon.metadata))
            {
                continue;
            }
            by_identifier
                .entry(Box::from(icon.identifier.as_ref()))
                .or_default()
                .insert(icon.metadata, sprites.len());
            sprites.push(IconSprite {
                width,
                height,
                rgba8: Arc::from(&icon.rgba8[..]),
            });
        }
        (!sprites.is_empty()).then(|| Self {
            atlas: SpriteAtlas::pack(&sprites),
            sprites,
            by_identifier,
        })
    }

    pub(crate) fn rasters(&self) -> &[EquipmentRaster] {
        &self.atlas.layers
    }
}

/// Installed session items: facts by identifier and placed icon sprites.
#[derive(Default)]
pub(super) struct SessionLayer {
    hand_equipped: BTreeMap<Box<str>, bool>,
    wearable: BTreeMap<Box<str>, ArmorSlot>,
    sprites: Vec<IconSprite>,
    by_identifier: BTreeMap<Box<str>, BTreeMap<u32, usize>>,
    placements: Vec<Option<Placement>>,
    locations: Vec<Option<ActorArtworkLocation>>,
}

/// The armor slot a `minecraft:wearable` slot name wears in.
pub(super) fn armor_slot(slot: &str) -> Option<ArmorSlot> {
    Some(match slot {
        "slot.armor.head" => ArmorSlot::Helmet,
        "slot.armor.chest" => ArmorSlot::Chestplate,
        "slot.armor.legs" => ArmorSlot::Leggings,
        "slot.armor.feet" => ArmorSlot::Boots,
        _ => return None,
    })
}

impl EquipmentRuntime {
    /// Installs the session's item facts and icons (`locations` parallel `icons.rasters()`), or
    /// clears them.
    pub(crate) fn set_session_items(
        &mut self,
        items: Option<&SessionItems>,
        icons: Option<StagedSessionIcons>,
        locations: Vec<Option<ActorArtworkLocation>>,
    ) {
        self.meshes
            .retain(|key, _| !matches!(key, MeshKey::Session(_)));
        let mut layer = SessionLayer::default();
        let mut item_use = (*self.base_item_use).clone();
        for (identifier, components) in items.into_iter().flat_map(|items| items.components.iter())
        {
            layer
                .hand_equipped
                .insert(Box::from(identifier.as_ref()), components.hand_equipped);
            if let Some(slot) = components.wearable_slot.as_deref().and_then(armor_slot) {
                layer.wearable.insert(Box::from(identifier.as_ref()), slot);
            }
            if let Some(ticks) = components.use_duration_ticks.filter(|ticks| *ticks > 0) {
                item_use.insert(Box::from(identifier.as_ref()), ticks);
            }
        }
        if let Some(icons) = icons {
            layer.sprites = icons.sprites;
            layer.by_identifier = icons.by_identifier;
            layer.placements = icons.atlas.placements;
            layer.locations = locations;
        }
        self.item_use = Arc::new(item_use);
        self.session = layer;
    }

    /// Whether `identifier` is held upright: a custom item's `hand_equipped` component, else
    /// vanilla's per-item choice.
    pub(super) fn hand_equipped(&self, identifier: &str) -> bool {
        match self.session.hand_equipped.get(identifier) {
            Some(&component) if !identifier.starts_with("minecraft:") => component,
            Some(&component) => component || is_hand_equipped(identifier),
            None => is_hand_equipped(identifier),
        }
    }

    /// The armor slot a custom item's `minecraft:wearable` names.
    pub(super) fn wearable_slot(&self, identifier: &str) -> Option<ArmorSlot> {
        self.session.wearable.get(identifier).copied()
    }

    /// The session icon for `identifier` with its atlas placement and artwork location.
    pub(super) fn session_sprite(
        &self,
        identifier: &str,
        metadata: u32,
    ) -> Option<(usize, Placement, ActorArtworkLocation)> {
        let variants = self.session.by_identifier.get(identifier)?;
        let index = *variants.get(&metadata).or_else(|| variants.get(&0))?;
        let placement = self.session.placements.get(index).copied().flatten()?;
        let location = self
            .session
            .locations
            .get(placement.layer)
            .copied()
            .flatten()?;
        Some((index, placement, location))
    }

    pub(super) fn session_sprite_pixels(&self, index: usize) -> Option<&IconSprite> {
        self.session.sprites.get(index)
    }
}

/// Mesh rig id of session icon `index`, above the startup item meshes.
pub(super) fn session_mesh_id(index: usize) -> Option<EntityRigId> {
    if index >= MAX_SESSION_MESHES {
        return None;
    }
    Some(item_mesh_rig_id(
        u32::try_from(MAX_ITEM_MESHES + index).ok()?,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wearable_slots_map_to_armor_slots_and_others_to_none() {
        assert_eq!(armor_slot("slot.armor.head"), Some(ArmorSlot::Helmet));
        assert_eq!(armor_slot("slot.armor.feet"), Some(ArmorSlot::Boots));
        assert_eq!(armor_slot("slot.weapon.offhand"), None);
    }

    // Session meshes stay in their own id range and refuse indices past it.
    #[test]
    fn session_mesh_ids_sit_above_the_startup_meshes() {
        assert_eq!(
            session_mesh_id(0),
            Some(item_mesh_rig_id(MAX_ITEM_MESHES as u32))
        );
        assert_eq!(session_mesh_id(MAX_SESSION_MESHES), None);
    }
}
