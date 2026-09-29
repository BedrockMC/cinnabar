//! Slot geometry of every container screen, shared by drawing and hit testing.
//!
//! Offsets are GUI pixels from the panel's top-left corner. Layout values come
//! from remembered public GUI texture layouts and need screenshot measurement.

use protocol::WindowKind;

use super::inventory_pointer::{InventoryCellHit, InventoryScreen};

mod creative;
mod window;

pub(crate) use creative::{
    CREATIVE_PANEL, GRID_CELLS, GRID_COLUMNS, GRID_ROWS, SEARCH_TAB, TAB_COUNT, creative_slots,
    tab_at, tab_origin, tab_size,
};

pub(crate) use window::{Widget, WindowLayout, widget_rects, window_layout};

pub(crate) const SLOT_SIZE: f32 = 18.0;
const PERSONAL_PANEL: [f32; 2] = [176.0, 166.0];
pub(crate) const WORKBENCH_GRID: [f32; 2] = [30.0, 17.0];
pub(crate) const WORKBENCH_OUTPUT: [f32; 2] = [124.0, 35.0];

/// One slot: its top-left corner and what a hit on it addresses.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct PlacedSlot {
    pub pos: [f32; 2],
    pub hit: InventoryCellHit,
    /// Result cells draw with the enlarged output frame.
    pub output: bool,
}

/// The panel's size in GUI pixels.
pub(crate) fn panel_size(screen: InventoryScreen) -> [f32; 2] {
    match screen {
        InventoryScreen::Storage(count) => [176.0, 114.0 + (count / 9) as f32 * SLOT_SIZE],
        InventoryScreen::Window(kind, cells) => {
            window_layout(kind, cells).map_or(PERSONAL_PANEL, |layout| layout.panel)
        }
        InventoryScreen::Creative => CREATIVE_PANEL,
        _ => PERSONAL_PANEL,
    }
}

/// The panel's top-left corner for a viewport of `gui` pixels.
pub(crate) fn panel_origin(screen: InventoryScreen, gui: [f32; 2]) -> [f32; 2] {
    let size = panel_size(screen);
    [
        ((gui[0] - size[0]) * 0.5).floor(),
        ((gui[1] - size[1]) * 0.5).floor(),
    ]
}

fn player_slots(x: f32, y: f32) -> impl Iterator<Item = PlacedSlot> {
    (0..27u8)
        .map(move |index| {
            let (row, column) = (index / 9, index % 9);
            PlacedSlot {
                pos: [
                    x + f32::from(column) * SLOT_SIZE,
                    y + f32::from(row) * SLOT_SIZE,
                ],
                hit: InventoryCellHit::Player(9 + index),
                output: false,
            }
        })
        .chain((0..9u8).map(move |column| PlacedSlot {
            pos: [x + f32::from(column) * SLOT_SIZE, y + 58.0],
            hit: InventoryCellHit::Player(column),
            output: false,
        }))
}

fn grid(first: u8, width: u8, at: [f32; 2]) -> impl Iterator<Item = PlacedSlot> {
    (0..width * width).map(move |index| PlacedSlot {
        pos: [
            at[0] + f32::from(index % width) * SLOT_SIZE,
            at[1] + f32::from(index / width) * SLOT_SIZE,
        ],
        hit: InventoryCellHit::Craft(first + index),
        output: false,
    })
}

/// Every slot of `screen` in panel-relative coordinates.
pub(crate) fn screen_slots(screen: InventoryScreen) -> Vec<PlacedSlot> {
    match screen {
        InventoryScreen::Personal => {
            let mut slots: Vec<PlacedSlot> = grid(28, 2, [98.0, 18.0]).collect();
            slots.push(PlacedSlot {
                pos: [152.0, 28.0],
                hit: InventoryCellHit::CraftOutput,
                output: false,
            });
            slots.extend((0..4u8).map(|row| PlacedSlot {
                pos: [8.0, 8.0 + f32::from(row) * SLOT_SIZE],
                hit: InventoryCellHit::Armor(row),
                output: false,
            }));
            slots.push(PlacedSlot {
                pos: [77.0, 62.0],
                hit: InventoryCellHit::Offhand,
                output: false,
            });
            slots.extend(player_slots(8.0, 84.0));
            slots
        }
        InventoryScreen::Workbench => {
            let mut slots: Vec<PlacedSlot> = grid(32, 3, WORKBENCH_GRID).collect();
            slots.push(PlacedSlot {
                pos: WORKBENCH_OUTPUT,
                hit: InventoryCellHit::CraftOutput,
                output: false,
            });
            slots.extend(player_slots(8.0, 84.0));
            slots
        }
        InventoryScreen::Storage(count) => {
            let rows = count / 9;
            let mut slots: Vec<PlacedSlot> = (0..count)
                .map(|index| PlacedSlot {
                    pos: [
                        8.0 + (index % 9) as f32 * SLOT_SIZE,
                        18.0 + (index / 9) as f32 * SLOT_SIZE,
                    ],
                    hit: InventoryCellHit::Storage(index as u8),
                    output: false,
                })
                .collect();
            slots.extend(player_slots(8.0, 32.0 + rows as f32 * SLOT_SIZE));
            slots
        }
        InventoryScreen::Window(kind, cells) => window_layout(kind, cells)
            .map(|layout| {
                let mut slots = layout.slots.clone();
                slots.extend(player_slots(layout.player[0], layout.player[1]));
                slots
            })
            .unwrap_or_default(),
        InventoryScreen::Creative => creative_slots(),
    }
}

/// The slot under `point` (panel-relative), if any.
pub(crate) fn slot_at(slots: &[PlacedSlot], point: [f32; 2]) -> Option<PlacedSlot> {
    slots.iter().copied().find(|slot| {
        let size = if slot.output { 26.0 } else { SLOT_SIZE };
        point[0] >= slot.pos[0]
            && point[0] < slot.pos[0] + size
            && point[1] >= slot.pos[1]
            && point[1] < slot.pos[1] + size
    })
}

/// Whether the screen shows a crafting-style output cell.
pub(crate) const fn has_output(kind: WindowKind) -> bool {
    matches!(
        kind,
        WindowKind::Anvil
            | WindowKind::Grindstone
            | WindowKind::Loom
            | WindowKind::Smithing
            | WindowKind::Cartography
            | WindowKind::Stonecutter
    )
}
