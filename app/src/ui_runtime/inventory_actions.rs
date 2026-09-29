//! Dispatch of resolved inventory-screen gestures onto the ledger: cells,
//! screen widgets, result cells and the creative catalog.

use protocol::{CreativeItem, WindowKind};

use super::UiRuntime;
use super::inventory_drag::PointerAction;
use super::inventory_ledger::{
    CellGesture, CraftSink, CreativeDestination, DistributeMode, InventoryGestureError,
    InventoryTarget, PlayerInventoryLedger, ScreenCraft,
};
use super::presentation::inventory_pointer::InventoryCellHit;
use super::presentation::screens::{
    BEACON_LEVEL_FOR, GRID_CELLS, GRID_COLUMNS, LOOM_COLUMNS, Widget,
};
use super::screen_recipes::LOOM_PATTERNS;
use super::screen_state::{ScreenState, creative_entries};

type Outcome = Result<i32, InventoryGestureError>;

/// The ledger target a cell hit addresses; output, widget and catalog hits have none.
pub(super) const fn gesture_target(hit: InventoryCellHit) -> Option<InventoryTarget> {
    Some(match hit {
        InventoryCellHit::Player(slot) => InventoryTarget::Player(slot),
        InventoryCellHit::Storage(slot) => InventoryTarget::Storage(slot),
        InventoryCellHit::Armor(slot) => InventoryTarget::Armor(slot),
        InventoryCellHit::Offhand => InventoryTarget::Offhand,
        InventoryCellHit::Craft(slot) => InventoryTarget::Craft(slot),
        InventoryCellHit::CraftOutput
        | InventoryCellHit::Widget(_)
        | InventoryCellHit::CreativeGrid(_)
        | InventoryCellHit::CreativeTab(_)
        | InventoryCellHit::CreativeSearch => return None,
    })
}

/// The catalog entries the creative screen currently lists, in grid order.
pub(crate) fn visible_creative_entries<'a>(
    ledger: &'a PlayerInventoryLedger,
    state: &ScreenState,
) -> Vec<&'a CreativeItem> {
    let Some(catalog) = ledger.creative_catalog() else {
        return Vec::new();
    };
    let name_of = |item: &CreativeItem| {
        let entry = ledger.negotiated_item_entry(item.stack.network_id)?;
        let name = entry.identifier.strip_prefix("minecraft:")?;
        Some(name.replace('_', " "))
    };
    creative_entries(catalog, state.creative_tab, &state.search, name_of)
}

impl UiRuntime {
    /// Runs one pointer action; refusals are ordinary (a busy or resyncing
    /// ledger) and simply drop the gesture.
    pub(crate) fn perform_pointer_action(&mut self, action: PointerAction) {
        let _ = match action {
            PointerAction::Click(hit) => self.click_hit(hit),
            PointerAction::SecondaryClick(hit) => self.secondary_click_hit(hit),
            PointerAction::QuickMove(hit) => self.quick_move_hit(hit),
            PointerAction::Distribute { cells, one_each } => {
                let targets: Vec<_> = cells.into_iter().filter_map(gesture_target).collect();
                let mode = if one_each {
                    DistributeMode::One
                } else {
                    DistributeMode::Even
                };
                self.inventory_ledger_mut().begin_distribute(&targets, mode)
            }
            PointerAction::Gather => self.inventory_ledger_mut().begin_gather(),
        };
    }

    fn click_hit(&mut self, hit: InventoryCellHit) -> Outcome {
        if hit != InventoryCellHit::Widget(Widget::AnvilName) {
            self.screen_state_mut().anvil_focused = false;
        }
        match hit {
            InventoryCellHit::Widget(widget) => self.activate_widget(widget),
            InventoryCellHit::CreativeTab(tab) => {
                self.screen_state_mut().select_tab(tab);
                Ok(0)
            }
            InventoryCellHit::CreativeSearch => {
                self.screen_state_mut()
                    .select_tab(super::presentation::screens::SEARCH_TAB);
                Ok(0)
            }
            InventoryCellHit::CreativeGrid(index) => self.creative_click(index, false),
            InventoryCellHit::CraftOutput => self.output_click(false),
            hit => match gesture_target(hit) {
                Some(target) => self
                    .inventory_ledger_mut()
                    .begin_target_gesture(target, CellGesture::Click),
                None => Err(InventoryGestureError::InvalidRequest),
            },
        }
    }

    fn secondary_click_hit(&mut self, hit: InventoryCellHit) -> Outcome {
        let Some(target) = gesture_target(hit) else {
            return Err(InventoryGestureError::InvalidRequest);
        };
        let ledger = self.inventory_ledger();
        let gesture = match (ledger.cursor_stack(), ledger.target_stack(target)) {
            (Some(_), _) => CellGesture::PlaceCount(1),
            (None, Some(stack)) => CellGesture::TakeCount(stack.count.div_ceil(2)),
            (None, None) => return Err(InventoryGestureError::EmptyGesture),
        };
        self.inventory_ledger_mut()
            .begin_target_gesture(target, gesture)
    }

    fn quick_move_hit(&mut self, hit: InventoryCellHit) -> Outcome {
        match hit {
            InventoryCellHit::CraftOutput => self.output_click(true),
            InventoryCellHit::CreativeGrid(index) => self.creative_click(index, true),
            hit => match gesture_target(hit) {
                Some(target) => self.inventory_ledger_mut().begin_quick_move(target),
                None => Err(InventoryGestureError::InvalidRequest),
            },
        }
    }

    /// Takes a result: the grid's recipe on the personal and crafting-table
    /// screens, the derived or previewed output on the others.
    fn output_click(&mut self, all: bool) -> Outcome {
        match self.inventory_ledger().window_kind() {
            None | Some(WindowKind::Workbench) => {
                if all {
                    self.begin_crafting_all()
                } else {
                    self.begin_crafting()
                }
            }
            Some(WindowKind::Anvil) => {
                let multi_recipe_id = self
                    .screen_catalog()
                    .and_then(|catalog| catalog.repair_multi_recipe_id())
                    .unwrap_or(0);
                let name = self.screen_state().anvil_name.trim();
                let rename = (!name.is_empty()).then(|| std::sync::Arc::from(name));
                self.inventory_ledger_mut()
                    .begin_screen_output(&ScreenCraft::Anvil {
                        rename,
                        multi_recipe_id,
                    })
            }
            Some(WindowKind::Grindstone) => {
                self.inventory_ledger_mut()
                    .begin_screen_output(&ScreenCraft::Grindstone {
                        recipe_network_id: 0,
                        repair_cost: 0,
                    })
            }
            Some(WindowKind::Loom) => {
                let pattern = self
                    .screen_state()
                    .loom_pattern
                    .clone()
                    .ok_or(InventoryGestureError::InvalidRequest)?;
                self.inventory_ledger_mut()
                    .begin_screen_output(&ScreenCraft::Loom { pattern })
            }
            Some(WindowKind::Stonecutter | WindowKind::Smithing | WindowKind::Cartography) => {
                let (recipe_network_id, output) = self
                    .active_screen_recipe()
                    .and_then(|recipe| Some((recipe.id, recipe.output?)))
                    .ok_or(InventoryGestureError::InvalidRequest)?;
                self.inventory_ledger_mut()
                    .begin_screen_output(&ScreenCraft::Predicted {
                        recipe_network_id,
                        output,
                    })
            }
            Some(_) => Err(InventoryGestureError::InvalidRequest),
        }
    }

    fn activate_widget(&mut self, widget: Widget) -> Outcome {
        match widget {
            Widget::EnchantOption(index) => {
                let id = self
                    .inventory_ledger()
                    .enchant_options()
                    .and_then(|options| options.get(usize::from(index)))
                    .map(|option| option.network_id)
                    .ok_or(InventoryGestureError::InvalidRequest)?;
                self.inventory_ledger_mut().begin_enchant(id)
            }
            Widget::BeaconEffect { id, secondary } => {
                let state = self.screen_state_mut();
                let unlocked = BEACON_LEVEL_FOR
                    .iter()
                    .find(|(effect, _)| *effect == id)
                    .is_some_and(|(_, needed)| {
                        state.beacon_level.is_none_or(|level| level >= *needed)
                    });
                if !unlocked {
                    return Err(InventoryGestureError::InvalidRequest);
                }
                if secondary {
                    state.beacon.1 = id;
                } else {
                    state.beacon.0 = id;
                }
                Ok(0)
            }
            Widget::BeaconUpgrade => {
                let state = self.screen_state_mut();
                if state.beacon.0 == 0 || state.beacon_level.is_some_and(|level| level < 4) {
                    return Err(InventoryGestureError::InvalidRequest);
                }
                state.beacon.1 = state.beacon.0;
                Ok(0)
            }
            Widget::BeaconConfirm => {
                let (primary, secondary) = self.screen_state().beacon;
                if primary == 0 {
                    return Err(InventoryGestureError::InvalidRequest);
                }
                self.inventory_ledger_mut()
                    .begin_beacon_payment(primary, secondary)
            }
            Widget::StonecutterRecipe(index) => {
                let id = self
                    .stonecutter_options()
                    .get(usize::from(index))
                    .map(|recipe| recipe.id)
                    .ok_or(InventoryGestureError::InvalidRequest)?;
                self.screen_state_mut().recipe_choice = Some(id);
                Ok(0)
            }
            Widget::LoomPattern(index) => {
                let position = self.screen_state().loom_row * LOOM_COLUMNS + usize::from(index);
                let pattern = LOOM_PATTERNS
                    .get(position)
                    .ok_or(InventoryGestureError::InvalidRequest)?;
                self.screen_state_mut().loom_pattern = Some(std::sync::Arc::from(*pattern));
                Ok(0)
            }
            Widget::AnvilName => {
                self.screen_state_mut().anvil_focused = true;
                Ok(0)
            }
        }
    }

    /// A click on a catalog cell: take the item, or delete the held stack.
    fn creative_click(&mut self, index: u8, into_inventory: bool) -> Outcome {
        if self.inventory_ledger().cursor_stack().is_some() {
            return self.inventory_ledger_mut().begin_destroy_cursor();
        }
        let id = {
            let entries = visible_creative_entries(self.inventory_ledger(), self.screen_state());
            let position = self.screen_state().creative_row * GRID_COLUMNS + usize::from(index);
            if usize::from(index) >= GRID_CELLS {
                return Err(InventoryGestureError::InvalidRequest);
            }
            entries
                .get(position)
                .map(|item| item.creative_network_id)
                .ok_or(InventoryGestureError::EmptyGesture)?
        };
        let destination = if into_inventory {
            let ledger = self.inventory_ledger();
            (0..protocol::PLAYER_INVENTORY_SLOTS)
                .find(|slot| {
                    ledger
                        .target_stack(InventoryTarget::Player(*slot))
                        .is_none()
                        && matches!(
                            ledger.slot_state(*slot),
                            Some(super::inventory_ledger::PlayerInventorySlot::Empty)
                        )
                })
                .map_or(CreativeDestination::Cursor, CreativeDestination::Player)
        } else {
            CreativeDestination::Cursor
        };
        self.inventory_ledger_mut()
            .begin_creative_take(id, destination)
    }

    /// Crafts the grid's recipe into hotbar `slot` while the pointer is over the result.
    pub(crate) fn craft_into_hotbar(&mut self, slot: u8) -> Outcome {
        self.begin_crafting_into(CraftSink::Player(slot))
    }
}
