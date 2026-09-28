//! Per-category equipment layer builders for a drawn body.

use super::*;

impl EquipmentRuntime {
    pub(super) fn push_held(
        &mut self,
        body: &ActorRigSubmission,
        item: &WornItem,
        layer: u8,
        hand: Option<usize>,
        layers: &mut Vec<EquipmentPresentation>,
    ) {
        if self.push_attachable(body, item, layer, hand, layer == LAYER_OFF_HAND, layers) {
            return;
        }
        self.push_attached(body, item, layer, hand, None, layers);
    }

    /// A held item whose attachable ships its own single-bone geometry and a literal
    /// third-person placement (trident, shield). `false` when the item is not one, or its
    /// placement is Molang-driven (then the sprite/cube path draws it).
    pub(super) fn push_attachable(
        &mut self,
        body: &ActorRigSubmission,
        item: &WornItem,
        layer: u8,
        hand: Option<usize>,
        off_hand: bool,
        layers: &mut Vec<EquipmentPresentation>,
    ) -> bool {
        let (Some(hand), Some(catalog)) = (hand, self.catalog.clone()) else {
            return false;
        };
        let Some(binding) = catalog.binding(&item.identifier) else {
            return false;
        };
        let channels = match binding.category {
            EquipmentCategory::Held => {
                binding
                    .third_person
                    .literal()
                    .map(|transform| BoneChannels {
                        translation: transform.translation.map(|value| value.get()),
                        rotation: transform.rotation.map(|value| value.get()),
                        scale: transform.scale.map(|value| value.get()),
                    })
            }
            EquipmentCategory::Shield => {
                let slot = if off_hand { "off_hand" } else { "main_hand" };
                binding
                    .pose(&format!("wield_third_person@{slot}"))
                    .and_then(|pose| pose.bones.first())
                    .map(|bone| {
                        let channel = |value: Option<[assets::ItemDisplayScalar; 3]>, rest: f32| {
                            value.map_or([rest; 3], |value| value.map(|scalar| scalar.get()))
                        };
                        BoneChannels {
                            translation: channel(bone.translation, 0.0),
                            rotation: channel(bone.rotation, 0.0),
                            scale: channel(bone.scale, 1.0),
                        }
                    })
            }
            _ => None,
        };
        let Some(channels) = channels else {
            return false;
        };
        let Some(location) = self
            .texture_locations
            .get(binding.texture.identifier.as_ref())
            .copied()
        else {
            return false;
        };
        let Some(geometry) = self.armor_geometry_for(&binding.geometry.identifier) else {
            return false;
        };
        // Only single-bone models are placed; a hierarchy needs its parent chain composed.
        let [pivot] = geometry.pivots[..] else {
            return false;
        };
        let (Some(previous), Some(current)) = (
            body.input.previous_bones.get(hand),
            body.input.current_bones.get(hand),
        ) else {
            return false;
        };
        let (Some(previous), Some(current)) = (
            attachable::attach(*previous, pivot, channels),
            attachable::attach(*current, pivot, channels),
        ) else {
            return false;
        };
        layers.push(layer_presentation(
            body,
            layer,
            geometry.rig,
            vec![previous],
            vec![current],
            location,
            0,
        ));
        true
    }

    /// A held or worn sprite/cube on `bone`, placed by `display` or the kind's held placement.
    pub(super) fn push_attached(
        &mut self,
        body: &ActorRigSubmission,
        item: &WornItem,
        layer: u8,
        bone: Option<usize>,
        override_display: Option<ItemDisplay>,
        layers: &mut Vec<EquipmentPresentation>,
    ) {
        let hand = bone;
        let Some(hand) = hand else {
            return;
        };
        let (sprite_index, key, display) = match item.kind {
            HeldKind::Sprite => {
                let Some(index) = self.icons.lookup_index(&item.identifier, item.metadata) else {
                    return;
                };
                (index, MeshKey::Sprite(index), held_sprite_display())
            }
            HeldKind::Block(visual) => {
                let Some(index) = self.block_sheets.get(&visual).copied() else {
                    return;
                };
                (index, MeshKey::Block(visual), held_block_display())
            }
            HeldKind::Other => return,
        };
        let display = override_display.unwrap_or(display);
        let Some(placement) = self.placements.get(sprite_index).copied().flatten() else {
            return;
        };
        let Some(location) = self.atlas_locations.get(placement.layer).copied().flatten() else {
            return;
        };
        let Some(mesh) = self.mesh_for(key, sprite_index, placement) else {
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
    pub(super) fn push_armor(
        &mut self,
        body: &ActorRigSubmission,
        bones: &BodyBones,
        body_geometry: u32,
        (slot, layer, stance): (ArmorSlot, u8, ElytraStance),
        item: &WornItem,
        layers: &mut Vec<EquipmentPresentation>,
    ) {
        let Some(catalog) = self.catalog.clone() else {
            return;
        };
        let Some(binding) = catalog.binding(&item.identifier) else {
            return;
        };
        let elytra_in_chest =
            binding.category == EquipmentCategory::Elytra && slot == ArmorSlot::Chestplate;
        if binding.category != (EquipmentCategory::Armor { slot }) && !elytra_in_chest {
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
        if elytra_in_chest {
            let Some(pose) = elytra::stance_pose(&binding, stance.sneaking, stance.sleeping) else {
                return;
            };
            let Some(body_index) = bones
                .names
                .iter()
                .position(|name| name.eq_ignore_ascii_case("body"))
            else {
                return;
            };
            let (Some(previous), Some(current)) = (
                body.input.previous_bones.get(body_index),
                body.input.current_bones.get(body_index),
            ) else {
                return;
            };
            layers.push(layer_presentation(
                body,
                layer,
                geometry.rig,
                elytra::pose(&geometry.names, pose, *previous),
                elytra::pose(&geometry.names, pose, *current),
                location,
                0,
            ));
            return;
        }
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

    /// A worn head riding the body's head bone.
    pub(super) fn push_skull(
        &mut self,
        body: &ActorRigSubmission,
        kind: SkullKind,
        layer: u8,
        head: Option<usize>,
        layers: &mut Vec<EquipmentPresentation>,
    ) {
        let Some(&(rig, location)) = self.skulls.get(&kind_index(kind)) else {
            return;
        };
        let Some(head) = head else {
            return;
        };
        let (Some(previous), Some(current)) = (
            body.input.previous_bones.get(head),
            body.input.current_bones.get(head),
        ) else {
            return;
        };
        layers.push(layer_presentation(
            body,
            layer,
            rig,
            vec![*previous],
            vec![*current],
            location,
            0,
        ));
    }
}
