use std::collections::BTreeMap;

use protocol::{ItemRegistryEntry, ItemRegistryEvent, ItemRegistryVersion, NetworkItemStack};
use sha2::{Digest, Sha256};

use super::{Cell, PlayerInventoryLedger};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum OccupiedStackRelation {
    Compatible { capacity: u16 },
    Incompatible,
    Unsupported,
}

impl PlayerInventoryLedger {
    pub(crate) fn negotiated_item_entry(&self, network_id: i32) -> Option<&ItemRegistryEntry> {
        self.item_registry.as_ref()?.get(&network_id)
    }

    pub fn apply_registry(&mut self, event: &ItemRegistryEvent) {
        let Some(next) = registry_map(event) else {
            return;
        };
        let Some(previous) = self.item_registry.as_ref() else {
            self.item_registry = Some(next);
            return;
        };
        if previous == &next {
            return;
        }

        let affected_cells = self.registry_affected_cells(previous, &next);
        let affected_requests: Vec<i32> = self
            .queue
            .iter()
            .filter(|pending| {
                pending
                    .predicted
                    .iter()
                    .map(|(_, held)| held.stack.network_id)
                    .any(|network_id| {
                        registry_identity_changed(previous, &next, network_id)
                            || (pending.registry_bound_merge
                                && registry_merge_rule_changed(previous, &next, network_id))
                    })
            })
            .map(|pending| pending.request_id)
            .collect();
        self.abandon_requests(|pending| affected_requests.contains(&pending.request_id));
        for cell in affected_cells {
            self.mark_cell_recovery(cell);
        }
        self.item_registry = Some(next);
        self.refold();
    }

    pub(super) fn occupied_stack_relation(
        &self,
        source: &NetworkItemStack,
        destination: &NetworkItemStack,
    ) -> OccupiedStackRelation {
        let distinct_network_ids = source.network_id != destination.network_id;
        if !plain_stack(source) || !plain_stack(destination) {
            return if distinct_network_ids {
                OccupiedStackRelation::Incompatible
            } else {
                OccupiedStackRelation::Unsupported
            };
        }
        if source.stack_network_id <= 0
            || destination.stack_network_id <= 0
            || source.stack_network_id == destination.stack_network_id
        {
            return OccupiedStackRelation::Unsupported;
        }
        let Some(registry) = self.item_registry.as_ref() else {
            return if distinct_network_ids {
                OccupiedStackRelation::Incompatible
            } else {
                OccupiedStackRelation::Unsupported
            };
        };
        let Some(source_entry) = registry.get(&source.network_id) else {
            return if distinct_network_ids {
                OccupiedStackRelation::Incompatible
            } else {
                OccupiedStackRelation::Unsupported
            };
        };
        let Some(destination_entry) = registry.get(&destination.network_id) else {
            return if distinct_network_ids {
                OccupiedStackRelation::Incompatible
            } else {
                OccupiedStackRelation::Unsupported
            };
        };
        if source_entry.identifier != destination_entry.identifier {
            return OccupiedStackRelation::Incompatible;
        }
        let Some(source_capacity) = entry_capacity(source_entry) else {
            return OccupiedStackRelation::Unsupported;
        };
        let Some(destination_capacity) = entry_capacity(destination_entry) else {
            return OccupiedStackRelation::Unsupported;
        };
        if source_capacity != destination_capacity {
            return OccupiedStackRelation::Unsupported;
        }
        OccupiedStackRelation::Compatible {
            capacity: u16::from(destination_capacity),
        }
    }

    fn registry_affected_cells(
        &self,
        previous: &BTreeMap<i32, ItemRegistryEntry>,
        next: &BTreeMap<i32, ItemRegistryEntry>,
    ) -> Vec<Cell> {
        self.confirmed
            .occupied()
            .filter(|(_, held)| registry_identity_changed(previous, next, held.stack.network_id))
            .map(|(cell, _)| cell)
            .collect()
    }
}

fn registry_map(event: &ItemRegistryEvent) -> Option<BTreeMap<i32, ItemRegistryEntry>> {
    if event.entries.len() > protocol::MAX_ITEM_REGISTRY_ENTRIES {
        return None;
    }
    let mut entries = BTreeMap::new();
    for entry in event.entries.iter() {
        if entries.insert(entry.network_id, entry.clone()).is_some() {
            return None;
        }
    }
    Some(entries)
}

fn registry_identity_changed(
    previous: &BTreeMap<i32, ItemRegistryEntry>,
    next: &BTreeMap<i32, ItemRegistryEntry>,
    network_id: i32,
) -> bool {
    match (previous.get(&network_id), next.get(&network_id)) {
        (Some(previous), Some(next)) => previous.identifier != next.identifier,
        (Some(_), None) => true,
        (None, Some(_)) | (None, None) => false,
    }
}

fn registry_merge_rule_changed(
    previous: &BTreeMap<i32, ItemRegistryEntry>,
    next: &BTreeMap<i32, ItemRegistryEntry>,
    network_id: i32,
) -> bool {
    effective_merge_binding(previous.get(&network_id))
        != effective_merge_binding(next.get(&network_id))
}

fn effective_merge_binding(entry: Option<&ItemRegistryEntry>) -> Option<(&str, Option<u8>)> {
    let entry = entry?;
    Some((entry.identifier.as_ref(), entry_capacity(entry)))
}

fn entry_capacity(entry: &ItemRegistryEntry) -> Option<u8> {
    if matches!(entry.version, ItemRegistryVersion::Unknown(_))
        || protocol::vanilla_item_capacity(&entry.identifier, 0).is_none()
    {
        return None;
    }
    if let Some(capacity) = entry.negotiated_max_stack_size {
        return Some(capacity);
    }
    if !entry.component_based && entry.canonical_empty_component_data {
        protocol::vanilla_item_capacity(&entry.identifier, 0)
    } else {
        None
    }
}

fn plain_stack(stack: &NetworkItemStack) -> bool {
    let digest: [u8; 32] = Sha256::digest(&stack.extra_data).into();
    stack.metadata == 0
        && stack.block_runtime_id == 0
        && (stack.extra_data.is_empty() || stack.extra_data.as_ref() == [0; 10])
        && stack.nbt_digest == digest
}
