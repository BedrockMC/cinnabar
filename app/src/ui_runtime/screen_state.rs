//! Transient state of the open inventory screen: hover, drag, creative tab,
//! search text and beacon effect choice.

use protocol::{CreativeCategory, CreativeContentEvent, CreativeItem};

use super::inventory_drag::InventoryPointer;
use super::presentation::inventory_pointer::InventoryCellHit;
use super::presentation::screens::{GRID_CELLS, GRID_COLUMNS, SEARCH_TAB};

/// Longest creative search text.
const MAX_SEARCH_CHARS: usize = 32;

#[derive(Debug, Default)]
pub(crate) struct ScreenState {
    pub(crate) pointer: InventoryPointer,
    pub(crate) hover: Option<InventoryCellHit>,
    /// Chosen beacon effects; `0` means none.
    pub(crate) beacon: (i32, i32),
    pub(crate) creative_tab: u8,
    /// First visible row of the creative grid.
    pub(crate) creative_row: usize,
    pub(crate) search: String,
    pub(crate) search_focused: bool,
    window: Option<u64>,
}

impl ScreenState {
    /// Clears per-window choices when a different window becomes current.
    pub(crate) fn observe_window(&mut self, generation: Option<u64>) {
        if self.window != generation {
            self.window = generation;
            self.pointer.reset();
            self.beacon = (0, 0);
            self.search_focused = false;
        }
    }

    pub(crate) fn select_tab(&mut self, tab: u8) {
        self.search_focused = tab == SEARCH_TAB;
        self.creative_tab = tab;
        self.creative_row = 0;
    }

    /// Appends typed text to the search field, dropping control characters.
    pub(crate) fn type_search(&mut self, text: &str) {
        for ch in text.chars().filter(|ch| !ch.is_control()) {
            if self.search.chars().count() < MAX_SEARCH_CHARS {
                self.search.push(ch);
            }
        }
        self.creative_row = 0;
    }

    pub(crate) fn backspace_search(&mut self) {
        self.search.pop();
        self.creative_row = 0;
    }

    /// Scrolls the creative grid by whole rows, keeping the last page reachable.
    pub(crate) fn scroll_creative(&mut self, rows: isize, total: usize) {
        let max_row = total
            .div_ceil(GRID_COLUMNS)
            .saturating_sub(GRID_CELLS / GRID_COLUMNS);
        self.creative_row = self.creative_row.saturating_add_signed(rows).min(max_row);
    }
}

fn tab_category(tab: u8) -> Option<CreativeCategory> {
    match tab {
        0 => Some(CreativeCategory::Construction),
        1 => Some(CreativeCategory::Nature),
        2 => Some(CreativeCategory::Equipment),
        3 => Some(CreativeCategory::Items),
        _ => None,
    }
}

/// The catalog entries a tab shows; the search tab shows every entry whose
/// name contains the text.
pub(crate) fn creative_entries<'a>(
    catalog: &'a CreativeContentEvent,
    tab: u8,
    search: &str,
    name_of: impl Fn(&CreativeItem) -> Option<String>,
) -> Vec<&'a CreativeItem> {
    let needle = search.to_lowercase();
    catalog
        .items
        .iter()
        .filter(|item| {
            let category = catalog
                .groups
                .get(item.group as usize)
                .map(|group| group.category);
            if category == Some(CreativeCategory::CommandOnly) {
                return false;
            }
            match tab_category(tab) {
                Some(wanted) => category == Some(wanted),
                None => {
                    needle.is_empty()
                        || name_of(item).is_some_and(|name| name.to_lowercase().contains(&needle))
                }
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use protocol::{CreativeGroup, NetworkItemStack};

    use super::*;

    fn catalog() -> CreativeContentEvent {
        let item = |id: u32, group: u32| CreativeItem {
            creative_network_id: id,
            stack: NetworkItemStack::default(),
            group,
        };
        CreativeContentEvent {
            groups: Arc::from([
                CreativeGroup {
                    category: CreativeCategory::Construction,
                    name: Arc::from("a"),
                },
                CreativeGroup {
                    category: CreativeCategory::Nature,
                    name: Arc::from("b"),
                },
                CreativeGroup {
                    category: CreativeCategory::CommandOnly,
                    name: Arc::from("c"),
                },
            ]),
            items: Arc::from([item(1, 0), item(2, 1), item(3, 2), item(4, 0)]),
            skipped: 0,
        }
    }

    #[test]
    fn tabs_filter_by_category_and_hide_command_only() {
        let catalog = catalog();
        let ids = |tab| {
            creative_entries(&catalog, tab, "", |_| None)
                .iter()
                .map(|item| item.creative_network_id)
                .collect::<Vec<_>>()
        };
        assert_eq!(ids(0), vec![1, 4]);
        assert_eq!(ids(1), vec![2]);
        assert_eq!(ids(SEARCH_TAB), vec![1, 2, 4]);
    }

    #[test]
    fn search_matches_names_case_insensitively() {
        let catalog = catalog();
        let found = creative_entries(&catalog, SEARCH_TAB, "STONE", |item| {
            (item.creative_network_id == 2).then(|| "Stone Brick".to_owned())
        });
        assert_eq!(found.len(), 1);
    }

    #[test]
    fn scrolling_stops_at_the_last_page() {
        let mut state = ScreenState::default();
        state.scroll_creative(50, 100);
        assert_eq!(state.creative_row, 100_usize.div_ceil(9) - 5);
        state.scroll_creative(-50, 100);
        assert_eq!(state.creative_row, 0);
    }
}
