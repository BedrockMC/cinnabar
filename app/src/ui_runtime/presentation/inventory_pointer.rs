use ui::UiPoint;

use super::{HudGeometry, UiPresentationRuntime};

const PANEL_SIZE: [f32; 2] = [176.0, 166.0];
const SLOT_SIZE: f32 = 18.0;

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub(crate) enum InventoryCellHit {
    Player(u8),
    Storage(u8),
    Armor(u8),
    Offhand,
    /// A crafting cell by UI inventory slot.
    Craft(u8),
    CraftOutput,
}

/// Which inventory screen is drawn.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub(crate) enum InventoryScreen {
    Personal,
    Workbench,
    Storage(usize),
}

impl InventoryScreen {
    pub(crate) fn of(ledger: &crate::ui_runtime::inventory_ledger::PlayerInventoryLedger) -> Self {
        use crate::ui_runtime::inventory_ledger::CraftingGrid;
        match (ledger.storage_slot_count(), ledger.crafting_grid()) {
            (Some(count @ (27 | 54)), _) => Self::Storage(count),
            (_, CraftingGrid::Workbench) => Self::Workbench,
            _ => Self::Personal,
        }
    }
}

/// Provisional workbench layout pending independent measurement: grid origin
/// and output cell relative to the panel.
pub(crate) const WORKBENCH_GRID: [f32; 2] = [30.0, 17.0];
pub(crate) const WORKBENCH_OUTPUT: [f32; 2] = [124.0, 35.0];

impl UiPresentationRuntime {
    pub(crate) fn inventory_gui_point(
        &self,
        point: UiPoint,
        physical_size: [u32; 2],
        dpi_scale: f32,
    ) -> Option<[f32; 2]> {
        let geometry = self.inventory_geometry(physical_size, dpi_scale)?;
        Some(gui_point(point, geometry, self.safe_area))
    }

    #[cfg(test)]
    pub(crate) fn inventory_slot_hit(
        &self,
        gui: [f32; 2],
        physical_size: [u32; 2],
        dpi_scale: f32,
    ) -> Option<u8> {
        let geometry = self.inventory_geometry(physical_size, dpi_scale)?;
        slot_hit(gui, geometry)
    }

    pub(crate) fn inventory_cell_hit(
        &self,
        gui: [f32; 2],
        physical_size: [u32; 2],
        dpi_scale: f32,
        screen: InventoryScreen,
    ) -> Option<InventoryCellHit> {
        let geometry = self.inventory_geometry(physical_size, dpi_scale)?;
        cell_hit(gui, geometry, screen)
    }

    /// Whether a GUI point lies on the drawn inventory panel.
    pub(crate) fn inventory_panel_contains(
        &self,
        gui: [f32; 2],
        physical_size: [u32; 2],
        dpi_scale: f32,
        screen: InventoryScreen,
    ) -> bool {
        let Some(geometry) = self.inventory_geometry(physical_size, dpi_scale) else {
            return false;
        };
        let height = match screen {
            InventoryScreen::Storage(count) => 114.0 + (count / 9) as f32 * SLOT_SIZE,
            InventoryScreen::Personal | InventoryScreen::Workbench => PANEL_SIZE[1],
        };
        let origin = [
            ((geometry.gui_width - PANEL_SIZE[0]) * 0.5).floor(),
            ((geometry.gui_height - height) * 0.5).floor(),
        ];
        gui[0] >= origin[0]
            && gui[0] < origin[0] + PANEL_SIZE[0]
            && gui[1] >= origin[1]
            && gui[1] < origin[1] + height
    }

    fn inventory_geometry(&self, physical_size: [u32; 2], dpi_scale: f32) -> Option<HudGeometry> {
        HudGeometry::new(
            physical_size,
            dpi_scale,
            self.safe_area,
            self.gui_scale_preference,
        )
    }
}

fn gui_point(point: UiPoint, geometry: HudGeometry, safe_area: ui::SafeArea) -> [f32; 2] {
    [
        (point.x() - safe_area.left()) / geometry.scale,
        (point.y() - safe_area.top()) / geometry.scale,
    ]
}

#[cfg(test)]
fn slot_hit(point: [f32; 2], geometry: HudGeometry) -> Option<u8> {
    match cell_hit(point, geometry, InventoryScreen::Personal)? {
        InventoryCellHit::Player(slot) => Some(slot),
        _ => None,
    }
}

fn cell_hit(
    point: [f32; 2],
    geometry: HudGeometry,
    screen: InventoryScreen,
) -> Option<InventoryCellHit> {
    if let InventoryScreen::Storage(count) = screen {
        let rows = count / 9;
        let panel_height = 114.0 + rows as f32 * SLOT_SIZE;
        let origin = [
            ((geometry.gui_width - PANEL_SIZE[0]) * 0.5).floor(),
            ((geometry.gui_height - panel_height) * 0.5).floor(),
        ];
        for slot in 0..count {
            let min = [
                origin[0] + 8.0 + (slot % 9) as f32 * SLOT_SIZE,
                origin[1] + 18.0 + (slot / 9) as f32 * SLOT_SIZE,
            ];
            if point_in_slot(point, min) {
                return Some(InventoryCellHit::Storage(slot as u8));
            }
        }
        let player_y = origin[1] + 32.0 + rows as f32 * SLOT_SIZE;
        for row in 0..3u8 {
            for column in 0..9u8 {
                if point_in_slot(
                    point,
                    [
                        origin[0] + 8.0 + f32::from(column) * SLOT_SIZE,
                        player_y + f32::from(row) * SLOT_SIZE,
                    ],
                ) {
                    return Some(InventoryCellHit::Player(9 + row * 9 + column));
                }
            }
        }
        for column in 0..9u8 {
            if point_in_slot(
                point,
                [
                    origin[0] + 8.0 + f32::from(column) * SLOT_SIZE,
                    player_y + 58.0,
                ],
            ) {
                return Some(InventoryCellHit::Player(column));
            }
        }
        return None;
    }
    let origin = [
        ((geometry.gui_width - PANEL_SIZE[0]) * 0.5).floor(),
        ((geometry.gui_height - PANEL_SIZE[1]) * 0.5).floor(),
    ];
    let at = |offset: [f32; 2]| [origin[0] + offset[0], origin[1] + offset[1]];
    if let Some(hit) = upper_hit(point, screen, at) {
        return Some(hit);
    }
    for row in 0..3u8 {
        for column in 0..9u8 {
            let min = [
                origin[0] + 8.0 + f32::from(column) * SLOT_SIZE,
                origin[1] + 84.0 + f32::from(row) * SLOT_SIZE,
            ];
            if point_in_slot(point, min) {
                return Some(InventoryCellHit::Player(9 + row * 9 + column));
            }
        }
    }
    for column in 0..9u8 {
        let min = [
            origin[0] + 8.0 + f32::from(column) * SLOT_SIZE,
            origin[1] + 142.0,
        ];
        if point_in_slot(point, min) {
            return Some(InventoryCellHit::Player(column));
        }
    }
    None
}

/// Cells above the player inventory: the grid the screen draws, its output,
/// and on the personal screen the armor column and offhand.
fn upper_hit(
    point: [f32; 2],
    screen: InventoryScreen,
    at: impl Fn([f32; 2]) -> [f32; 2],
) -> Option<InventoryCellHit> {
    let (grid, width, first_slot, output) = match screen {
        InventoryScreen::Workbench => (WORKBENCH_GRID, 3u8, 32u8, WORKBENCH_OUTPUT),
        _ => ([98.0, 18.0], 2, 28, [152.0, 28.0]),
    };
    for index in 0..width * width {
        let cell = [
            grid[0] + f32::from(index % width) * SLOT_SIZE,
            grid[1] + f32::from(index / width) * SLOT_SIZE,
        ];
        if point_in_slot(point, at(cell)) {
            return Some(InventoryCellHit::Craft(first_slot + index));
        }
    }
    if point_in_slot(point, at(output)) {
        return Some(InventoryCellHit::CraftOutput);
    }
    if screen != InventoryScreen::Personal {
        return None;
    }
    for row in 0..4u8 {
        if point_in_slot(point, at([8.0, 8.0 + f32::from(row) * SLOT_SIZE])) {
            return Some(InventoryCellHit::Armor(row));
        }
    }
    point_in_slot(point, at([77.0, 62.0])).then_some(InventoryCellHit::Offhand)
}

fn point_in_slot(point: [f32; 2], min: [f32; 2]) -> bool {
    point[0] >= min[0]
        && point[0] < min[0] + SLOT_SIZE
        && point[1] >= min[1]
        && point[1] < min[1] + SLOT_SIZE
}

#[cfg(test)]
mod tests {
    use super::*;
    use ui::SafeArea;

    fn geometry(physical: [u32; 2], dpi: f32, safe: SafeArea) -> HudGeometry {
        HudGeometry::new(physical, dpi, safe, Some(2)).expect("valid inventory geometry")
    }

    #[test]
    fn dpi_and_safe_area_conversion_retains_pointer_outside_slots() {
        let safe = SafeArea::new(20.0, 10.0, 0.0, 0.0).unwrap();
        let geometry = geometry([1920, 1080], 2.0, safe);
        let point = UiPoint::new(420.0, 210.0).unwrap();
        assert_eq!(gui_point(point, geometry, safe), [400.0, 200.0]);
        assert_eq!(slot_hit([0.0, 0.0], geometry), None);
        assert_eq!(
            slot_hit([geometry.gui_width, geometry.gui_height], geometry),
            None
        );
    }

    #[test]
    fn only_the_36_player_cells_are_interactive() {
        let geometry = geometry([1280, 720], 1.0, SafeArea::ZERO);
        let origin = [
            ((geometry.gui_width - PANEL_SIZE[0]) * 0.5).floor(),
            ((geometry.gui_height - PANEL_SIZE[1]) * 0.5).floor(),
        ];
        assert_eq!(
            slot_hit([origin[0] + 9.0, origin[1] + 85.0], geometry),
            Some(9)
        );
        assert_eq!(
            slot_hit([origin[0] + 153.0, origin[1] + 143.0], geometry),
            Some(8)
        );
        assert_eq!(
            slot_hit([origin[0] + 9.0, origin[1] + 139.0], geometry),
            None
        );
        assert_eq!(slot_hit([origin[0] + 9.0, origin[1] + 9.0], geometry), None);
        assert_eq!(
            slot_hit([origin[0] + 99.0, origin[1] + 19.0], geometry),
            None
        );
        assert_eq!(
            slot_hit([origin[0] - 1.0, origin[1] + 85.0], geometry),
            None
        );
    }

    #[test]
    fn generic_storage_hit_testing_is_exact_for_27_and_54_cells() {
        let geometry = geometry([1280, 720], 1.0, SafeArea::ZERO);
        for count in [27, 54] {
            let rows = count / 9;
            let panel_height = 114.0 + rows as f32 * SLOT_SIZE;
            let origin = [
                ((geometry.gui_width - PANEL_SIZE[0]) * 0.5).floor(),
                ((geometry.gui_height - panel_height) * 0.5).floor(),
            ];
            assert_eq!(
                cell_hit(
                    [origin[0] + 9.0, origin[1] + 19.0],
                    geometry,
                    InventoryScreen::Storage(count)
                ),
                Some(InventoryCellHit::Storage(0))
            );
            let last = count - 1;
            assert_eq!(
                cell_hit(
                    [
                        origin[0] + 9.0 + (last % 9) as f32 * SLOT_SIZE,
                        origin[1] + 19.0 + (last / 9) as f32 * SLOT_SIZE,
                    ],
                    geometry,
                    InventoryScreen::Storage(count),
                ),
                Some(InventoryCellHit::Storage(last as u8))
            );
            assert_eq!(
                cell_hit(
                    [origin[0] + 9.0, origin[1] + 33.0 + rows as f32 * SLOT_SIZE],
                    geometry,
                    InventoryScreen::Storage(count),
                ),
                Some(InventoryCellHit::Player(9))
            );
        }
    }

    /// Grids, output, armor and offhand resolve to their own cells on each
    /// screen; the workbench offers no equipment cells.
    #[test]
    fn crafting_and_equipment_cells_resolve_per_screen() {
        let geometry = geometry([1280, 720], 1.0, SafeArea::ZERO);
        let origin = [
            ((geometry.gui_width - PANEL_SIZE[0]) * 0.5).floor(),
            ((geometry.gui_height - PANEL_SIZE[1]) * 0.5).floor(),
        ];
        let hit = |offset: [f32; 2], screen| {
            cell_hit(
                [origin[0] + offset[0] + 1.0, origin[1] + offset[1] + 1.0],
                geometry,
                screen,
            )
        };
        let personal = InventoryScreen::Personal;
        assert_eq!(
            hit([98.0, 18.0], personal),
            Some(InventoryCellHit::Craft(28))
        );
        assert_eq!(
            hit([116.0, 36.0], personal),
            Some(InventoryCellHit::Craft(31))
        );
        assert_eq!(
            hit([152.0, 28.0], personal),
            Some(InventoryCellHit::CraftOutput)
        );
        assert_eq!(hit([8.0, 62.0], personal), Some(InventoryCellHit::Armor(3)));
        assert_eq!(hit([77.0, 62.0], personal), Some(InventoryCellHit::Offhand));
        let workbench = InventoryScreen::Workbench;
        assert_eq!(
            hit(WORKBENCH_GRID, workbench),
            Some(InventoryCellHit::Craft(32))
        );
        assert_eq!(
            hit(
                [WORKBENCH_GRID[0] + 36.0, WORKBENCH_GRID[1] + 36.0],
                workbench
            ),
            Some(InventoryCellHit::Craft(40))
        );
        assert_eq!(
            hit(WORKBENCH_OUTPUT, workbench),
            Some(InventoryCellHit::CraftOutput)
        );
        assert_eq!(hit([8.0, 8.0], workbench), None);
        assert_eq!(
            hit([8.0, 84.0], workbench),
            Some(InventoryCellHit::Player(9))
        );
    }
}
