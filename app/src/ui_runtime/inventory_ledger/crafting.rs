//! Crafting-table requests over the personal 2x2 or workbench 3x3 grid,
//! ported from the owner's proxy prediction: every ingredient must claim a
//! distinct grid cell or nothing is predicted.

use std::sync::Arc;

use protocol::{CraftGridItem, CraftResult, NetworkItemStack, RecipeHandle, StackRequestAction};
use sha2::{Digest, Sha256};

use super::cells::{Cell, FIRST_CRAFT_SLOT, Held};
use super::gesture::Submission;
use super::overlay::DeltaGroup;
use super::registry::{entry_capacity, plain_stack};
use super::{InventoryGestureError, PlayerInventoryLedger, WORKBENCH_WINDOW_TYPE, helpers};

/// Which crafting grid the open screen offers.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum CraftingGrid {
    /// The personal 2x2 grid, UI slots 28..=31.
    Personal,
    /// The workbench 3x3 grid, UI slots 32..=40.
    Workbench,
}

impl CraftingGrid {
    #[must_use]
    pub const fn width(self) -> u8 {
        match self {
            Self::Personal => 2,
            Self::Workbench => 3,
        }
    }

    /// The UI inventory slot of each grid cell in row-major order.
    pub fn slots(self) -> impl Iterator<Item = u8> {
        let (first, count) = match self {
            Self::Personal => (FIRST_CRAFT_SLOT, 4),
            Self::Workbench => (FIRST_CRAFT_SLOT + 4, 9),
        };
        first..first + count
    }
}

/// Where a creative take lands.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum CreativeDestination {
    Cursor,
    /// An empty player cell.
    Player(u8),
}

/// One presented grid cell resolved through the session item registry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CraftGridCell {
    pub identifier: Arc<str>,
    pub metadata: u32,
    pub count: u16,
    pub plain: bool,
}

impl CraftGridCell {
    #[must_use]
    pub fn item(&self) -> CraftGridItem<'_> {
        CraftGridItem {
            identifier: &self.identifier,
            metadata: self.metadata,
            count: self.count,
            plain: self.plain,
        }
    }
}

impl PlayerInventoryLedger {
    #[must_use]
    pub fn crafting_grid(&self) -> CraftingGrid {
        if self
            .storage
            .as_ref()
            .is_some_and(|storage| storage.window_type == WORKBENCH_WINDOW_TYPE)
        {
            CraftingGrid::Workbench
        } else {
            CraftingGrid::Personal
        }
    }

    /// The presented grid, or `None` while any occupied cell's item identity
    /// is unknown to the session registry.
    #[must_use]
    pub fn crafting_grid_cells(&self) -> Option<Vec<Option<CraftGridCell>>> {
        self.crafting_grid()
            .slots()
            .map(|slot| match self.view().get(Cell::Craft(slot)) {
                None => Some(None),
                Some(held) => Some(Some(CraftGridCell {
                    identifier: Arc::clone(
                        &self
                            .negotiated_item_entry(held.stack.network_id)?
                            .identifier,
                    ),
                    metadata: held.stack.metadata,
                    count: held.stack.count,
                    plain: plain_stack(&held.stack),
                })),
            })
            .collect()
    }

    /// Crafts `recipe` once per `crafts` into an empty cursor as one request:
    /// CraftRecipe, CraftResultsDeprecated, one Consume per claimed cell, and
    /// a Take from created output named by this request's id.
    pub fn begin_craft(
        &mut self,
        recipe: &RecipeHandle,
        crafts: u8,
    ) -> Result<i32, InventoryGestureError> {
        let personal_generation = self.gesture_preflight(true)?;
        self.check_surfaces([Cell::Cursor, Cell::CreatedOutput])?;
        if crafts == 0 || self.view().get(Cell::Cursor).is_some() {
            return Err(InventoryGestureError::InvalidRequest);
        }
        let cells = self
            .crafting_grid_cells()
            .ok_or(InventoryGestureError::InvalidRequest)?;
        let slots: Vec<u8> = self.crafting_grid().slots().collect();
        let mut claimed = vec![false; slots.len()];
        let mut consumed = Vec::new();
        for (index, per_craft) in recipe.ingredient_counts().enumerate() {
            let cell = (0..slots.len())
                .find(|cell| {
                    !claimed[*cell]
                        && cells[*cell].as_ref().is_some_and(|grid| {
                            recipe.ingredient_accepts(index, &grid.item(), crafts)
                        })
                })
                .ok_or(InventoryGestureError::InvalidRequest)?;
            claimed[cell] = true;
            let held = self
                .view()
                .get(Cell::Craft(slots[cell]))
                .expect("claimed cells are occupied");
            if held.stack.stack_network_id <= 0 || self.awaiting_identity(held) {
                return Err(InventoryGestureError::AwaitingIdentity);
            }
            let amount = u8::try_from(u16::from(per_craft) * u16::from(crafts))
                .map_err(|_| InventoryGestureError::InvalidRequest)?;
            consumed.push((
                Cell::Craft(slots[cell]),
                amount,
                held.stack.stack_network_id,
            ));
        }
        let output = recipe.output();
        let entry = self
            .negotiated_item_entry(output.network_id)
            .ok_or(InventoryGestureError::InvalidRequest)?;
        let total = u8::try_from(u16::from(output.count) * u16::from(crafts))
            .ok()
            .filter(|total| entry_capacity(entry).is_some_and(|capacity| *total <= capacity))
            .ok_or(InventoryGestureError::InvalidRequest)?;
        let request_id = self.peek_request_id()?;
        let user_data: Arc<[u8]> = if output.empty_envelope {
            Arc::from([0; 10])
        } else {
            Arc::from([])
        };
        let created = NetworkItemStack {
            network_id: output.network_id,
            metadata: u32::from(output.aux),
            stack_network_id: request_id,
            count: u16::from(total),
            nbt_digest: Sha256::digest(&user_data).into(),
            block_runtime_id: i32::try_from(output.block_runtime_id)
                .map_err(|_| InventoryGestureError::InvalidRequest)?,
            extra_data: Arc::clone(&user_data),
        };
        let mut actions = vec![
            StackRequestAction::CraftRecipe {
                recipe_network_id: recipe.network_id(),
                crafts,
            },
            StackRequestAction::CraftResultsDeprecated {
                results: Arc::from([CraftResult {
                    identifier: Arc::clone(&entry.identifier),
                    aux: i32::from(output.aux),
                    count: u16::from(output.count),
                    block_runtime_id: output.block_runtime_id,
                    user_data,
                }]),
                crafts,
            },
        ];
        let mut groups = Vec::new();
        for (cell, amount, id) in consumed {
            actions.push(StackRequestAction::Consume {
                amount,
                source: helpers::request_slot(cell, id, None)?,
            });
            groups.push(DeltaGroup::Shrink {
                source: cell,
                amount: u16::from(amount),
                source_id: id,
            });
        }
        actions.push(StackRequestAction::Take {
            amount: total,
            source: helpers::request_slot(Cell::CreatedOutput, request_id, None)?,
            destination: helpers::request_slot(Cell::Cursor, 0, None)?,
        });
        groups.push(DeltaGroup::Set {
            cell: Cell::CreatedOutput,
            held: Held {
                stack: created,
                overlay: None,
            },
        });
        groups.push(DeltaGroup::Transfer {
            source: Cell::CreatedOutput,
            destination: Cell::Cursor,
            amount: u16::from(total),
            source_id: request_id,
            destination_id: None,
            capacity: None,
        });
        self.submit(Submission {
            actions,
            groups,
            personal_generation,
            // The crafted stack must settle under a real server id.
            requires_distinct_stack_ids: true,
            registry_bound_merge: false,
        })
    }

    /// The server's current creative catalog.
    #[must_use]
    pub fn creative_catalog(&self) -> Option<&protocol::CreativeContentEvent> {
        self.creative.as_ref()
    }

    /// Takes a full stack of one creative entry into an empty destination:
    /// CraftCreative, CraftResultsDeprecated, then a transfer from created
    /// output named by this request's id.
    pub fn begin_creative_take(
        &mut self,
        creative_network_id: u32,
        destination: CreativeDestination,
    ) -> Result<i32, InventoryGestureError> {
        let personal_generation = self.gesture_preflight(true)?;
        let item = self
            .creative
            .as_ref()
            .and_then(|catalog| catalog.item(creative_network_id))
            .ok_or(InventoryGestureError::InvalidRequest)?;
        // The client takes a full stack even though entries advertise one.
        let full = self
            .negotiated_item_entry(item.stack.network_id)
            .and_then(entry_capacity)
            .ok_or(InventoryGestureError::InvalidRequest)?;
        let target = match destination {
            CreativeDestination::Cursor => Cell::Cursor,
            CreativeDestination::Player(slot) => {
                if !self.known.get(usize::from(slot)).copied().unwrap_or(false) {
                    return Err(InventoryGestureError::UnknownSlot(slot));
                }
                Cell::Inventory(slot)
            }
        };
        self.check_surfaces([target, Cell::CreatedOutput])?;
        if self.view().get(target).is_some() {
            return Err(InventoryGestureError::InvalidRequest);
        }
        let request_id = self.peek_request_id()?;
        let mut stack = item.stack.clone();
        stack.count = u16::from(full);
        stack.stack_network_id = request_id;
        let source = helpers::request_slot(Cell::CreatedOutput, request_id, None)?;
        let destination_slot = helpers::request_slot(target, 0, None)?;
        let transfer = match destination {
            CreativeDestination::Cursor => StackRequestAction::Take {
                amount: full,
                source,
                destination: destination_slot,
            },
            CreativeDestination::Player(_) => StackRequestAction::Place {
                amount: full,
                source,
                destination: destination_slot,
            },
        };
        self.submit(Submission {
            actions: vec![
                StackRequestAction::CraftCreative {
                    creative_item_network_id: creative_network_id,
                    crafts: 1,
                },
                StackRequestAction::CraftResultsDeprecated {
                    results: Arc::from([]),
                    crafts: 1,
                },
                transfer,
            ],
            groups: vec![
                DeltaGroup::Set {
                    cell: Cell::CreatedOutput,
                    held: Held {
                        stack,
                        overlay: None,
                    },
                },
                DeltaGroup::Transfer {
                    source: Cell::CreatedOutput,
                    destination: target,
                    amount: u16::from(full),
                    source_id: request_id,
                    destination_id: None,
                    capacity: None,
                },
            ],
            personal_generation,
            requires_distinct_stack_ids: true,
            registry_bound_merge: false,
        })
    }
}
