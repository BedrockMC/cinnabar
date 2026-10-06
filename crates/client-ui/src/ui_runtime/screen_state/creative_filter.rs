//! Retained creative search results follow the catalog, registry and query.

use std::{collections::BTreeMap, sync::Arc};

use protocol::{CreativeContentEvent, CreativeItem, ItemRegistryEntry};

use super::{ScreenState, creative_entry_indexes};
use crate::ui_runtime::inventory_ledger::PlayerInventoryLedger;

#[derive(Clone, Debug)]
pub(super) struct CreativeFilterCache {
    catalog: CreativeContentEvent,
    registry: Option<Arc<BTreeMap<i32, ItemRegistryEntry>>>,
    tab: u8,
    search: String,
    indexes: Vec<usize>,
}

impl ScreenState {
    pub(crate) fn matching_creative_entries<'a>(
        &self,
        ledger: &'a PlayerInventoryLedger,
        name_of: impl Fn(&CreativeItem) -> Option<String>,
    ) -> Vec<&'a CreativeItem> {
        let Some(catalog) = ledger.creative_catalog() else {
            return Vec::new();
        };
        let registry = ledger.item_registry_snapshot();
        let mut cache = self
            .creative_filter
            .lock()
            .expect("creative filter lock poisoned");
        let reusable = cache.as_ref().is_some_and(|cached| {
            Arc::ptr_eq(&cached.catalog.items, &catalog.items)
                && Arc::ptr_eq(&cached.catalog.groups, &catalog.groups)
                && cached.tab == self.creative_tab
                && cached.search == self.search
                && match (cached.registry.as_ref(), registry) {
                    (Some(previous), Some(current)) => Arc::ptr_eq(previous, current),
                    (None, None) => true,
                    _ => false,
                }
        });
        if !reusable {
            *cache = Some(CreativeFilterCache {
                catalog: catalog.clone(),
                registry: registry.cloned(),
                tab: self.creative_tab,
                search: self.search.clone(),
                indexes: creative_entry_indexes(catalog, self.creative_tab, &self.search, name_of),
            });
        }
        cache
            .as_ref()
            .expect("the current creative filter was retained")
            .indexes
            .iter()
            .map(|index| &catalog.items[*index])
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use protocol::{
        CreativeCategory, CreativeGroup, InventoryEvent, ItemRegistryEvent, NetworkItemStack,
    };
    use std::cell::Cell;

    #[test]
    fn creative_search_only_rescans_when_its_inputs_change() {
        let mut ledger = PlayerInventoryLedger::default();
        let catalog = CreativeContentEvent {
            groups: Arc::from([CreativeGroup {
                category: CreativeCategory::Nature,
                name: Arc::from(""),
                icon: None,
            }]),
            items: Arc::from([CreativeItem {
                creative_network_id: 1,
                stack: NetworkItemStack::default(),
                group: 0,
            }]),
            skipped: 0,
        };
        ledger.apply(&InventoryEvent::Creative(catalog.clone()));
        let mut state = ScreenState::default();
        state.creative_tab = crate::ui_runtime::presentation::screens::SEARCH_TAB;
        state.search = "stone".into();
        let scans = Cell::new(0);
        let name = |_: &CreativeItem| {
            scans.set(scans.get() + 1);
            Some("Stone".into())
        };
        assert_eq!(state.matching_creative_entries(&ledger, name).len(), 1);
        for _ in 0..3 {
            assert_eq!(state.matching_creative_entries(&ledger, name).len(), 1);
        }
        assert_eq!(scans.get(), 1);
        state.search = "dirt".into();
        assert!(state.matching_creative_entries(&ledger, name).is_empty());
        assert_eq!(scans.get(), 2);
        ledger.apply_registry(&ItemRegistryEvent {
            entries: Arc::from([]),
        });
        assert!(state.matching_creative_entries(&ledger, name).is_empty());
        assert_eq!(scans.get(), 3);
        ledger.apply(&InventoryEvent::Creative(CreativeContentEvent {
            items: catalog.items.to_vec().into(),
            ..catalog
        }));
        assert!(state.matching_creative_entries(&ledger, name).is_empty());
        assert_eq!(scans.get(), 4);
    }
}
