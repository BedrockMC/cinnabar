use std::{
    collections::{BTreeMap, HashMap, VecDeque},
    sync::Arc,
};

use assets::{
    BlockVisualId, ItemStackIdentity, ItemVisualDefinitionRoute, ItemVisualId, ItemVisualKey,
    ItemVisualRoute, RuntimeEntityAssets,
};
use protocol::{
    ActorHandedness, ArmorEquipmentEvent, EquipmentEvent, ItemRegistryEntry, ItemRegistryEvent,
    ItemRegistryVersion, NetworkItemStack,
};
use sha2::{Digest, Sha256};

use crate::{ActorEventIdentity, ActorLifetimeId, ActorSourceTick};

pub const MAX_ITEM_REGISTRY_RECORDS: usize = 16_384;
pub const MAX_PENDING_ITEM_RESOLUTIONS: usize = 1_024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CanonicalItemStack {
    pub identity: ItemStackIdentity,
    pub identifier: Option<Arc<str>>,
    pub visual: ItemVisualRoute,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CanonicalItemRegistryRecord {
    pub identifier: Arc<str>,
    pub network_id: i32,
    pub component_based: bool,
    pub version: ItemRegistryVersion,
    pub component_digest: [u8; 32],
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActorEquipmentSnapshot {
    pub actor: ActorLifetimeId,
    pub event: ActorEventIdentity,
    pub item: CanonicalItemStack,
    pub inventory_slot: i32,
    pub selected_slot: u8,
    pub window_id: u8,
    pub hand: ActorHandedness,
    pub hand_defaulted: bool,
}

/// One worn armor stack with the dye colour its NBT carries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActorArmorPiece {
    pub item: CanonicalItemStack,
    /// Leather dye RGB (24-bit) from the stack's `customColor` tag.
    pub dye_rgb: Option<u32>,
}

/// An actor's five worn armor stacks from its latest MobArmorEquipment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActorArmorSnapshot {
    /// Lifetime the stacks were applied to; `spawn_revision` 0 when the actor did not exist yet.
    pub actor: ActorLifetimeId,
    pub event: ActorEventIdentity,
    pub helmet: ActorArmorPiece,
    pub chestplate: ActorArmorPiece,
    pub leggings: ActorArmorPiece,
    pub boots: ActorArmorPiece,
    pub body: ActorArmorPiece,
}

type EquipmentKey = (ActorLifetimeId, ActorHandedness);

#[derive(Debug)]
pub(crate) struct ItemStateStore {
    assets: Option<Arc<RuntimeEntityAssets>>,
    registry: BTreeMap<i32, CanonicalItemRegistryRecord>,
    equipment: BTreeMap<EquipmentKey, ActorEquipmentSnapshot>,
    pending: VecDeque<EquipmentKey>,
    /// Latest armor by runtime id; the client-owned runtime survives actor churn.
    armor: BTreeMap<u64, ActorArmorSnapshot>,
    persistent_armor_runtime: Option<u64>,
}

impl ItemStateStore {
    pub(crate) fn diagnostic() -> Self {
        Self::new(None)
    }

    pub(crate) fn with_assets(assets: Arc<RuntimeEntityAssets>) -> Self {
        Self::new(Some(assets))
    }

    fn new(assets: Option<Arc<RuntimeEntityAssets>>) -> Self {
        Self {
            assets,
            registry: built_in_registry(),
            equipment: BTreeMap::new(),
            pending: VecDeque::new(),
            armor: BTreeMap::new(),
            persistent_armor_runtime: None,
        }
    }

    /// Keeps this runtime's armor across actor removal and dimension resets (the local player).
    pub(crate) fn set_persistent_armor_runtime(&mut self, runtime_id: u64) {
        self.persistent_armor_runtime = Some(runtime_id);
    }

    #[cfg(test)]
    pub(crate) fn clear(&mut self) {
        self.registry.clear();
        self.armor.clear();
        self.clear_actor_state();
    }

    pub(crate) fn clear_actor_state(&mut self) {
        self.equipment.clear();
        self.pending.clear();
        let persistent = self.persistent_armor_runtime;
        self.armor
            .retain(|runtime_id, _| Some(*runtime_id) == persistent);
    }

    pub(crate) fn remove(&mut self, lifetime: ActorLifetimeId) {
        self.equipment.retain(|(actor, _), _| *actor != lifetime);
        self.pending.retain(|(actor, _)| *actor != lifetime);
        if self.persistent_armor_runtime != Some(lifetime.runtime_id) {
            self.armor.remove(&lifetime.runtime_id);
        }
    }

    pub(crate) fn insert_spawn(
        &mut self,
        lifetime: ActorLifetimeId,
        sequence: u64,
        stack: NetworkItemStack,
    ) {
        self.remove_runtime(lifetime.runtime_id);
        let Some(item) = self.canonicalize(&stack) else {
            return;
        };
        let unresolved = !item.identity.is_empty() && item.identifier.is_none();
        let key = (lifetime, ActorHandedness::Right);
        self.equipment.insert(
            key,
            ActorEquipmentSnapshot {
                actor: lifetime,
                event: event_identity(
                    lifetime,
                    sequence,
                    ActorSourceTick::IngressSequence(sequence),
                ),
                item,
                inventory_slot: -1,
                selected_slot: 0,
                window_id: u8::MAX,
                hand: ActorHandedness::Right,
                hand_defaulted: true,
            },
        );
        if unresolved {
            self.retain_pending(key);
        }
    }

    pub(crate) fn apply_equipment(
        &mut self,
        lifetime: ActorLifetimeId,
        sequence: u64,
        equipment: EquipmentEvent,
    ) -> bool {
        let Some(item) = self.canonicalize(&equipment.stack) else {
            return false;
        };
        let unresolved = !item.identity.is_empty() && item.identifier.is_none();
        let (hand, hand_defaulted) = equipment
            .handedness
            .map_or((ActorHandedness::Right, true), |hand| (hand, false));
        let key = (lifetime, hand);
        self.equipment.insert(
            key,
            ActorEquipmentSnapshot {
                actor: lifetime,
                event: event_identity(
                    lifetime,
                    sequence,
                    ActorSourceTick::IngressSequence(sequence),
                ),
                item,
                inventory_slot: equipment.inventory_slot,
                selected_slot: equipment.selected_slot,
                window_id: equipment.window_id,
                hand,
                hand_defaulted,
            },
        );
        self.pending.retain(|pending| *pending != key);
        if unresolved {
            self.retain_pending(key);
        }
        true
    }

    /// Stores all five worn stacks, or rejects the event if any stack's NBT digest is wrong.
    pub(crate) fn apply_armor(
        &mut self,
        lifetime: ActorLifetimeId,
        sequence: u64,
        event: &ArmorEquipmentEvent,
    ) -> bool {
        let (Some(helmet), Some(chestplate), Some(leggings), Some(boots), Some(body)) = (
            self.armor_piece(&event.helmet),
            self.armor_piece(&event.chestplate),
            self.armor_piece(&event.leggings),
            self.armor_piece(&event.boots),
            self.armor_piece(&event.body),
        ) else {
            return false;
        };
        self.armor.insert(
            lifetime.runtime_id,
            ActorArmorSnapshot {
                actor: lifetime,
                event: event_identity(
                    lifetime,
                    sequence,
                    ActorSourceTick::IngressSequence(sequence),
                ),
                helmet,
                chestplate,
                leggings,
                boots,
                body,
            },
        );
        true
    }

    pub(crate) fn armor(&self, runtime_id: u64) -> Option<&ActorArmorSnapshot> {
        self.armor.get(&runtime_id)
    }

    fn armor_piece(&self, stack: &NetworkItemStack) -> Option<ActorArmorPiece> {
        Some(ActorArmorPiece {
            item: self.canonicalize(stack)?,
            dye_rgb: protocol::item_custom_color(&stack.extra_data),
        })
    }

    pub(crate) fn apply_registry(&mut self, registry: ItemRegistryEvent) -> bool {
        if registry.entries.len() > MAX_ITEM_REGISTRY_RECORDS {
            return false;
        }
        let mut next = built_in_registry();
        let mut identifiers = HashMap::with_capacity(registry.entries.len());
        let mut network_ids = HashMap::with_capacity(registry.entries.len());
        for entry in registry.entries.iter() {
            if network_ids.insert(entry.network_id, ()).is_some()
                || identifiers
                    .insert(Arc::clone(&entry.identifier), ())
                    .is_some()
            {
                return false;
            }
            next.insert(entry.network_id, registry_record(entry));
        }
        self.registry = next;

        let runtimes = self.armor.keys().copied().collect::<Vec<_>>();
        for runtime_id in runtimes {
            let Some(mut snapshot) = self.armor.remove(&runtime_id) else {
                continue;
            };
            for piece in [
                &mut snapshot.helmet,
                &mut snapshot.chestplate,
                &mut snapshot.leggings,
                &mut snapshot.boots,
                &mut snapshot.body,
            ] {
                piece.item = self.resolve_identity(piece.item.identity);
            }
            self.armor.insert(runtime_id, snapshot);
        }

        let keys = self.equipment.keys().copied().collect::<Vec<_>>();
        self.pending.clear();
        for key in keys {
            let Some(identity) = self
                .equipment
                .get(&key)
                .map(|equipment| equipment.item.identity)
            else {
                continue;
            };
            let item = self.resolve_identity(identity);
            let unresolved = !item.identity.is_empty() && item.identifier.is_none();
            if let Some(equipment) = self.equipment.get_mut(&key) {
                equipment.item = item;
            }
            if unresolved {
                self.retain_pending(key);
            }
        }
        true
    }

    pub(crate) fn get(&self, lifetime: ActorLifetimeId) -> Option<&ActorEquipmentSnapshot> {
        [ActorHandedness::Left, ActorHandedness::Right]
            .into_iter()
            .filter_map(|hand| self.get_in_hand(lifetime, hand))
            .max_by_key(|equipment| equipment.event.ingress_sequence)
    }

    pub(crate) fn get_in_hand(
        &self,
        lifetime: ActorLifetimeId,
        hand: ActorHandedness,
    ) -> Option<&ActorEquipmentSnapshot> {
        self.equipment.get(&(lifetime, hand))
    }

    pub(crate) fn pending_count(&self) -> usize {
        self.pending.len()
    }

    pub(crate) fn canonicalize(&self, stack: &NetworkItemStack) -> Option<CanonicalItemStack> {
        let digest: [u8; 32] = Sha256::digest(stack.extra_data.as_ref()).into();
        if digest != stack.nbt_digest {
            return None;
        }
        let identity = ItemStackIdentity {
            network_id: stack.network_id,
            metadata: stack.metadata,
            stack_network_id: stack.stack_network_id,
            count: stack.count,
            nbt_digest: stack.nbt_digest,
            block_runtime_id: stack.block_runtime_id,
        };
        let identity = if identity.count == 0 {
            ItemStackIdentity::empty()
        } else if identity.network_id == 0 {
            return None;
        } else {
            identity
        };
        Some(self.resolve_identity(identity))
    }

    /// The registry identifier for an item network id.
    pub(crate) fn identifier_for_network_id(&self, network_id: i32) -> Option<Arc<str>> {
        self.registry
            .get(&network_id)
            .map(|record| Arc::clone(&record.identifier))
    }

    fn resolve_identity(&self, identity: ItemStackIdentity) -> CanonicalItemStack {
        if identity.is_empty() {
            return CanonicalItemStack {
                identity,
                identifier: None,
                visual: ItemVisualRoute::EmptyHand,
            };
        }
        let identifier = self
            .registry
            .get(&identity.network_id)
            .map(|record| Arc::clone(&record.identifier));
        let visual = classify_retained_block(
            identifier
                .as_deref()
                .map_or(ItemVisualRoute::Missing, |identifier| {
                    self.resolve_visual(identifier, identity.metadata)
                }),
            identity.block_runtime_id,
        );
        CanonicalItemStack {
            identity,
            identifier,
            visual,
        }
    }

    /// Visual route for an item identifier with no stack context (e.g. the TNT block).
    pub(crate) fn visual_for_identifier(&self, identifier: &str) -> ItemVisualRoute {
        self.resolve_visual(identifier, 0)
    }

    fn resolve_visual(&self, identifier: &str, metadata: u32) -> ItemVisualRoute {
        let Some(assets) = self.assets.as_ref() else {
            return ItemVisualRoute::Missing;
        };
        let key = ItemVisualKey {
            identifier: identifier.into(),
            metadata,
        };
        if let Ok(index) = assets
            .item_visuals()
            .binary_search_by(|visual| visual.key.cmp(&key))
        {
            return match assets.item_visuals()[index].route {
                ItemVisualDefinitionRoute::Sprite { .. } => {
                    ItemVisualRoute::Compiled(ItemVisualId(index as u32))
                }
                ItemVisualDefinitionRoute::BlockItem { block_visual } => {
                    ItemVisualRoute::BlockItem(BlockVisualId(block_visual.0))
                }
                ItemVisualDefinitionRoute::EmptyHand => ItemVisualRoute::EmptyHand,
                ItemVisualDefinitionRoute::Missing => ItemVisualRoute::Missing,
            };
        }
        assets
            .item_visual_aliases()
            .binary_search_by(|alias| alias.key.cmp(&key))
            .ok()
            .map_or(ItemVisualRoute::Missing, |index| {
                ItemVisualRoute::Compiled(assets.item_visual_aliases()[index].visual)
            })
    }

    fn retain_pending(&mut self, key: EquipmentKey) {
        if self.pending.len() < MAX_PENDING_ITEM_RESOLUTIONS && !self.pending.contains(&key) {
            self.pending.push_back(key);
        }
    }

    fn remove_runtime(&mut self, runtime_id: u64) {
        self.equipment
            .retain(|(lifetime, _), _| lifetime.runtime_id != runtime_id);
        self.pending
            .retain(|(lifetime, _)| lifetime.runtime_id != runtime_id);
        if self.persistent_armor_runtime != Some(runtime_id) {
            self.armor.remove(&runtime_id);
        }
    }
}

fn built_in_registry() -> BTreeMap<i32, CanonicalItemRegistryRecord> {
    protocol::vanilla_item_registry()
        .iter()
        .map(|entry| (entry.network_id, registry_record(entry)))
        .collect()
}

/// Routes a stack that retained block runtime identity onto the explicit
/// block-item marker, keeping compiled block-item geometry authoritative and
/// leaving stacks without a retained identity exactly as resolved.
///
/// Classification reads only wire-retained fields; it never infers geometry,
/// textures, or identity from item or file names.
fn classify_retained_block(route: ItemVisualRoute, block_runtime_id: i32) -> ItemVisualRoute {
    if block_runtime_id == 0 || matches!(route, ItemVisualRoute::BlockItem(_)) {
        return route;
    }
    ItemVisualRoute::RetainedBlock { block_runtime_id }
}

fn registry_record(entry: &ItemRegistryEntry) -> CanonicalItemRegistryRecord {
    CanonicalItemRegistryRecord {
        identifier: Arc::clone(&entry.identifier),
        network_id: entry.network_id,
        component_based: entry.component_based,
        version: entry.version,
        component_digest: entry.component_digest,
    }
}

fn event_identity(
    actor: ActorLifetimeId,
    ingress_sequence: u64,
    source_tick: ActorSourceTick,
) -> ActorEventIdentity {
    ActorEventIdentity {
        session_id: actor.session_id,
        dimension: actor.dimension,
        actor_lifetime: actor.spawn_revision,
        ingress_sequence,
        source_tick,
    }
}

#[cfg(test)]
mod armor_tests {
    use super::*;

    fn lifetime(runtime_id: u64, spawn_revision: u64) -> ActorLifetimeId {
        ActorLifetimeId {
            session_id: 1,
            dimension: 0,
            runtime_id,
            spawn_revision,
        }
    }

    fn stack(network_id: i32, extra: &[u8]) -> NetworkItemStack {
        NetworkItemStack {
            network_id,
            metadata: 0,
            stack_network_id: -1,
            count: 1,
            nbt_digest: Sha256::digest(extra).into(),
            block_runtime_id: 0,
            extra_data: Arc::from(extra),
        }
    }

    fn dyed_extra() -> Vec<u8> {
        let mut encoded = vec![0xff, 0xff, 0x01, 0x0a, 0x00, 0x00, 0x03];
        encoded.extend_from_slice(&11u16.to_le_bytes());
        encoded.extend_from_slice(b"customColor");
        encoded.extend_from_slice(&0x0033_66ccu32.to_le_bytes());
        encoded.push(0x00);
        encoded
    }

    fn event(runtime_id: u64, helmet: NetworkItemStack) -> ArmorEquipmentEvent {
        ArmorEquipmentEvent {
            actor_runtime_id: runtime_id,
            helmet,
            chestplate: NetworkItemStack::empty(),
            leggings: NetworkItemStack::empty(),
            boots: NetworkItemStack::empty(),
            body: NetworkItemStack::empty(),
        }
    }

    #[test]
    fn armor_keeps_dye_and_drops_with_the_actor_unless_persistent() {
        let mut store = ItemStateStore::diagnostic();
        let extra = dyed_extra();
        assert!(store.apply_armor(lifetime(7, 1), 1, &event(7, stack(1, &extra))));
        assert!(store.apply_armor(lifetime(8, 1), 2, &event(8, stack(1, &extra))));
        assert_eq!(store.armor(7).unwrap().helmet.dye_rgb, Some(0x0033_66cc));
        assert!(store.armor(7).unwrap().chestplate.item.identity.is_empty());

        store.set_persistent_armor_runtime(8);
        store.remove(lifetime(7, 1));
        store.remove(lifetime(8, 1));
        assert!(store.armor(7).is_none());
        assert!(store.armor(8).is_some());
        store.clear_actor_state();
        assert!(store.armor(8).is_some());
    }

    #[test]
    fn armor_with_a_wrong_nbt_digest_is_rejected_whole() {
        let mut store = ItemStateStore::diagnostic();
        let mut bad = stack(1, &dyed_extra());
        bad.nbt_digest = [9; 32];
        assert!(!store.apply_armor(lifetime(7, 1), 1, &event(7, bad)));
        assert!(store.armor(7).is_none());
    }
}
