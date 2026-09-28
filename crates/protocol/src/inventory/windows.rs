//! Vanilla window types and how each screen's cells are addressed on the wire.
//!
//! Two backings exist: a window's own inventory (furnace, hopper, chest, ...)
//! whose cells follow content order, and the personal UI inventory (anvil,
//! enchanting table, ...) whose cells sit at fixed UI slots.

use super::container_policy::CONTAINER_NAME_CREATED_OUTPUT;
use super::request::StackRequestContainer;

pub const WINDOW_TYPE_CONTAINER: i8 = 0;
pub const WINDOW_TYPE_WORKBENCH: i8 = 1;
pub const WINDOW_TYPE_FURNACE: i8 = 2;
pub const WINDOW_TYPE_ENCHANTMENT: i8 = 3;
pub const WINDOW_TYPE_BREWING_STAND: i8 = 4;
pub const WINDOW_TYPE_ANVIL: i8 = 5;
pub const WINDOW_TYPE_DISPENSER: i8 = 6;
pub const WINDOW_TYPE_DROPPER: i8 = 7;
pub const WINDOW_TYPE_HOPPER: i8 = 8;
pub const WINDOW_TYPE_CART_CHEST: i8 = 10;
pub const WINDOW_TYPE_CART_HOPPER: i8 = 11;
pub const WINDOW_TYPE_HORSE: i8 = 12;
pub const WINDOW_TYPE_BEACON: i8 = 13;
pub const WINDOW_TYPE_LOOM: i8 = 24;
pub const WINDOW_TYPE_GRINDSTONE: i8 = 26;
pub const WINDOW_TYPE_BLAST_FURNACE: i8 = 27;
pub const WINDOW_TYPE_SMOKER: i8 = 28;
pub const WINDOW_TYPE_STONECUTTER: i8 = 29;
pub const WINDOW_TYPE_CARTOGRAPHY: i8 = 30;
pub const WINDOW_TYPE_SMITHING_TABLE: i8 = 33;
pub const WINDOW_TYPE_CHEST_BOAT: i8 = 34;
pub const WINDOW_TYPE_CRAFTER: i8 = 36;

const NAME_ANVIL_INPUT: u8 = 0;
const NAME_ANVIL_MATERIAL: u8 = 1;
const NAME_SMITHING_INPUT: u8 = 3;
const NAME_SMITHING_MATERIAL: u8 = 4;
const NAME_BEACON_PAYMENT: u8 = 8;
const NAME_BREWING_INPUT: u8 = 9;
const NAME_BREWING_RESULT: u8 = 10;
const NAME_BREWING_FUEL: u8 = 11;
const NAME_CRAFTING_INPUT: u8 = 13;
const NAME_ENCHANTING_INPUT: u8 = 22;
const NAME_ENCHANTING_MATERIAL: u8 = 23;
const NAME_FURNACE_FUEL: u8 = 24;
const NAME_FURNACE_INGREDIENT: u8 = 25;
const NAME_FURNACE_RESULT: u8 = 26;
const NAME_HORSE_EQUIP: u8 = 27;
const NAME_SHULKER_BOX: u8 = 30;
const NAME_LOOM_INPUT: u8 = 41;
const NAME_LOOM_DYE: u8 = 42;
const NAME_LOOM_MATERIAL: u8 = 43;
const NAME_BLAST_FURNACE_INGREDIENT: u8 = 45;
const NAME_SMOKER_INGREDIENT: u8 = 46;
const NAME_GRINDSTONE_INPUT: u8 = 50;
const NAME_GRINDSTONE_ADDITIONAL: u8 = 51;
const NAME_STONECUTTER_INPUT: u8 = 53;
const NAME_CARTOGRAPHY_INPUT: u8 = 55;
const NAME_CARTOGRAPHY_ADDITIONAL: u8 = 56;
const NAME_BARREL: u8 = 58;
const NAME_SMITHING_TEMPLATE: u8 = 61;
const NAME_CRAFTER: u8 = 62;

/// Result-preview container names; each shows the screen's output at UI slot 50.
const RESULT_PREVIEW_NAMES: [u8; 8] = [2, 5, 14, 44, 52, 54, 57, 33];

/// Slots of the personal UI inventory (cursor, inputs, grids, output).
pub const UI_SLOT_COUNT: usize = 54;

/// The screen a container window opens.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum WindowKind {
    /// Chest-like generic storage (chest, barrel, shulker box, ender chest, minecart, boat).
    Storage,
    Workbench,
    Furnace,
    BlastFurnace,
    Smoker,
    Enchanting,
    Brewing,
    Anvil,
    Dispenser,
    Dropper,
    Hopper,
    Horse,
    Beacon,
    Loom,
    Grindstone,
    Stonecutter,
    Cartography,
    Smithing,
    Crafter,
}

/// One run of a window's own cells that share a container name.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct WindowSegment {
    pub name: u8,
    pub first: u8,
    /// `0` runs to the last cell.
    pub count: u8,
}

/// How a window's own inventory is laid out.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum OpenCells {
    /// Chest-like: any listed length, requests name the level-entity container.
    Generic(&'static [usize]),
    /// Fixed named runs; content must be one of the listed lengths.
    Named {
        lengths: &'static [usize],
        segments: &'static [WindowSegment],
    },
}

const fn seg(name: u8, first: u8, count: u8) -> WindowSegment {
    WindowSegment { name, first, count }
}

const FURNACE_SEGMENTS: [WindowSegment; 3] = [
    seg(NAME_FURNACE_INGREDIENT, 0, 1),
    seg(NAME_FURNACE_FUEL, 1, 1),
    seg(NAME_FURNACE_RESULT, 2, 1),
];
const BLAST_SEGMENTS: [WindowSegment; 3] = [
    seg(NAME_BLAST_FURNACE_INGREDIENT, 0, 1),
    seg(NAME_FURNACE_FUEL, 1, 1),
    seg(NAME_FURNACE_RESULT, 2, 1),
];
const SMOKER_SEGMENTS: [WindowSegment; 3] = [
    seg(NAME_SMOKER_INGREDIENT, 0, 1),
    seg(NAME_FURNACE_FUEL, 1, 1),
    seg(NAME_FURNACE_RESULT, 2, 1),
];
const BREWING_SEGMENTS: [WindowSegment; 3] = [
    seg(NAME_BREWING_INPUT, 0, 1),
    seg(NAME_BREWING_RESULT, 1, 3),
    seg(NAME_BREWING_FUEL, 4, 1),
];
const HORSE_SEGMENTS: [WindowSegment; 2] = [seg(NAME_HORSE_EQUIP, 0, 2), seg(7, 2, 0)];
const CRAFTER_SEGMENTS: [WindowSegment; 1] = [seg(NAME_CRAFTER, 0, 0)];

impl WindowKind {
    /// The screen for a `ContainerOpen` window type; `None` for types with no screen here.
    #[must_use]
    pub const fn from_window_type(window_type: i8) -> Option<Self> {
        Some(match window_type {
            WINDOW_TYPE_CONTAINER | WINDOW_TYPE_CART_CHEST | WINDOW_TYPE_CHEST_BOAT => {
                Self::Storage
            }
            WINDOW_TYPE_WORKBENCH => Self::Workbench,
            WINDOW_TYPE_FURNACE => Self::Furnace,
            WINDOW_TYPE_BLAST_FURNACE => Self::BlastFurnace,
            WINDOW_TYPE_SMOKER => Self::Smoker,
            WINDOW_TYPE_ENCHANTMENT => Self::Enchanting,
            WINDOW_TYPE_BREWING_STAND => Self::Brewing,
            WINDOW_TYPE_ANVIL => Self::Anvil,
            WINDOW_TYPE_DISPENSER => Self::Dispenser,
            WINDOW_TYPE_DROPPER => Self::Dropper,
            WINDOW_TYPE_HOPPER | WINDOW_TYPE_CART_HOPPER => Self::Hopper,
            WINDOW_TYPE_HORSE => Self::Horse,
            WINDOW_TYPE_BEACON => Self::Beacon,
            WINDOW_TYPE_LOOM => Self::Loom,
            WINDOW_TYPE_GRINDSTONE => Self::Grindstone,
            WINDOW_TYPE_STONECUTTER => Self::Stonecutter,
            WINDOW_TYPE_CARTOGRAPHY => Self::Cartography,
            WINDOW_TYPE_SMITHING_TABLE => Self::Smithing,
            WINDOW_TYPE_CRAFTER => Self::Crafter,
            _ => return None,
        })
    }

    /// The window's own cells, or `None` when its cells live in the UI inventory.
    #[must_use]
    pub const fn open_cells(self) -> Option<OpenCells> {
        Some(match self {
            Self::Storage => OpenCells::Generic(&[27, 54]),
            Self::Dispenser | Self::Dropper => OpenCells::Generic(&[9]),
            Self::Hopper => OpenCells::Generic(&[5]),
            Self::Furnace => OpenCells::Named {
                lengths: &[3],
                segments: &FURNACE_SEGMENTS,
            },
            Self::BlastFurnace => OpenCells::Named {
                lengths: &[3],
                segments: &BLAST_SEGMENTS,
            },
            Self::Smoker => OpenCells::Named {
                lengths: &[3],
                segments: &SMOKER_SEGMENTS,
            },
            Self::Brewing => OpenCells::Named {
                lengths: &[5],
                segments: &BREWING_SEGMENTS,
            },
            // Saddle and armor, then the chest a donkey, mule or llama carries.
            Self::Horse => OpenCells::Named {
                lengths: &[2, 17, 5, 8, 11, 14],
                segments: &HORSE_SEGMENTS,
            },
            Self::Crafter => OpenCells::Named {
                lengths: &[9],
                segments: &CRAFTER_SEGMENTS,
            },
            Self::Workbench
            | Self::Enchanting
            | Self::Anvil
            | Self::Beacon
            | Self::Loom
            | Self::Grindstone
            | Self::Stonecutter
            | Self::Cartography
            | Self::Smithing => return None,
        })
    }

    /// Whether the window keeps its cells in the personal UI inventory.
    #[must_use]
    pub const fn is_ui_backed(self) -> bool {
        self.open_cells().is_none()
    }

    /// Lengths an `InventoryContent` for this window's own cells may have.
    #[must_use]
    pub const fn content_lengths(self) -> &'static [usize] {
        match self.open_cells() {
            Some(OpenCells::Generic(lengths) | OpenCells::Named { lengths, .. }) => lengths,
            None => &[],
        }
    }
}

/// The container name and window index request slot `cell` of an open window
/// carries. `content_name` is the name the window's content arrived under, so a
/// barrel or shulker box echoes its own name instead of the level-entity one.
#[must_use]
pub fn open_cell_request(
    kind: WindowKind,
    cell: u8,
    dynamic_id: Option<u32>,
    content_name: Option<u8>,
) -> Option<(StackRequestContainer, u8)> {
    match kind.open_cells()? {
        OpenCells::Generic(_) => Some(match content_name {
            Some(name @ (NAME_SHULKER_BOX | NAME_BARREL)) => {
                (StackRequestContainer::OpenWindow { name, dynamic_id }, cell)
            }
            _ => (StackRequestContainer::LevelEntity { dynamic_id }, cell),
        }),
        OpenCells::Named { segments, .. } => {
            let segment = segment_of_cell(segments, cell)?;
            let container = if segment.name == super::address::CONTAINER_NAME_LEVEL_ENTITY {
                StackRequestContainer::LevelEntity { dynamic_id }
            } else {
                StackRequestContainer::OpenWindow {
                    name: segment.name,
                    dynamic_id: None,
                }
            };
            Some((container, cell))
        }
    }
}

fn segment_of_cell(segments: &[WindowSegment], cell: u8) -> Option<WindowSegment> {
    segments
        .iter()
        .rev()
        .find(|segment| cell >= segment.first)
        .copied()
        .filter(|segment| segment.count == 0 || cell < segment.first + segment.count)
}

/// The first open-window cell a named run starts at, for content or slot
/// updates addressed by `name`; `None` when the window has no such run.
#[must_use]
pub fn open_name_first_cell(kind: WindowKind, name: u8) -> Option<u8> {
    match kind.open_cells()? {
        OpenCells::Generic(_) => matches!(
            name,
            super::address::CONTAINER_NAME_LEVEL_ENTITY | NAME_SHULKER_BOX | NAME_BARREL
        )
        .then_some(0),
        OpenCells::Named { segments, .. } => segments
            .iter()
            .find(|segment| segment.name == name)
            .map(|segment| segment.first),
    }
}

/// Whether `name` is a chest-like container name that maps onto generic storage.
#[must_use]
pub const fn is_chest_like_name(name: u8) -> bool {
    matches!(name, NAME_SHULKER_BOX | NAME_BARREL)
}

/// Whether `name` addresses a cell of a named open window.
#[must_use]
pub const fn is_open_window_name(name: u8) -> bool {
    matches!(
        name,
        NAME_BREWING_INPUT
            | NAME_BREWING_RESULT
            | NAME_BREWING_FUEL
            | NAME_FURNACE_FUEL
            | NAME_FURNACE_INGREDIENT
            | NAME_FURNACE_RESULT
            | NAME_HORSE_EQUIP
            | NAME_BLAST_FURNACE_INGREDIENT
            | NAME_SMOKER_INGREDIENT
            | NAME_CRAFTER
    )
}

/// The container name vanilla gives UI inventory slot `slot`, or `None` for a
/// slot no screen uses.
#[must_use]
pub const fn ui_slot_container_name(slot: u8) -> Option<u8> {
    Some(match slot {
        1 => NAME_ANVIL_INPUT,
        2 => NAME_ANVIL_MATERIAL,
        3 => NAME_STONECUTTER_INPUT,
        9 => NAME_LOOM_INPUT,
        10 => NAME_LOOM_DYE,
        11 => NAME_LOOM_MATERIAL,
        12 => NAME_CARTOGRAPHY_INPUT,
        13 => NAME_CARTOGRAPHY_ADDITIONAL,
        14 => NAME_ENCHANTING_INPUT,
        15 => NAME_ENCHANTING_MATERIAL,
        16 => NAME_GRINDSTONE_INPUT,
        17 => NAME_GRINDSTONE_ADDITIONAL,
        27 => NAME_BEACON_PAYMENT,
        28..=40 => NAME_CRAFTING_INPUT,
        50 => CONTAINER_NAME_CREATED_OUTPUT,
        51 => NAME_SMITHING_INPUT,
        52 => NAME_SMITHING_MATERIAL,
        53 => NAME_SMITHING_TEMPLATE,
        _ => return None,
    })
}

/// Whether `name` is a result-preview container that mirrors UI slot 50.
#[must_use]
pub fn is_result_preview_name(name: u8) -> bool {
    RESULT_PREVIEW_NAMES.contains(&name)
}

/// The UI slot a named UI container addresses at `slot`, or `None` when the
/// name and slot disagree.
#[must_use]
pub fn ui_slot_for_name(name: u8, slot: u16) -> Option<u8> {
    let slot = u8::try_from(slot).ok()?;
    (ui_slot_container_name(slot)? == name).then_some(slot)
}

/// The request container for UI inventory slot `slot`.
#[must_use]
pub fn ui_slot_request_container(slot: u8) -> Option<StackRequestContainer> {
    Some(match ui_slot_container_name(slot)? {
        NAME_CRAFTING_INPUT => StackRequestContainer::CraftingInput,
        CONTAINER_NAME_CREATED_OUTPUT => StackRequestContainer::CreatedOutput,
        name => StackRequestContainer::OpenWindow {
            name,
            dynamic_id: None,
        },
    })
}

#[cfg(test)]
mod tests {
    use valentine::bedrock::codec::BedrockCodec;
    use valentine::bedrock::version::v1_26_44::EnumsContainerEnumName as Name;

    use super::*;

    fn code(name: Name) -> u8 {
        let mut bytes = bytes::BytesMut::new();
        name.encode(&mut bytes).expect("a container name encodes");
        bytes[0]
    }

    /// Every hard-coded container name matches the pinned protocol enum.
    #[test]
    fn container_name_codes_match_the_protocol_enum() {
        let pairs = [
            (NAME_ANVIL_INPUT, Name::AnvilInputContainer),
            (NAME_ANVIL_MATERIAL, Name::AnvilMaterialContainer),
            (NAME_SMITHING_INPUT, Name::SmithingTableInputContainer),
            (NAME_SMITHING_MATERIAL, Name::SmithingTableMaterialContainer),
            (NAME_BEACON_PAYMENT, Name::BeaconPaymentContainer),
            (NAME_BREWING_INPUT, Name::BrewingStandInputContainer),
            (NAME_BREWING_RESULT, Name::BrewingStandResultContainer),
            (NAME_BREWING_FUEL, Name::BrewingStandFuelContainer),
            (NAME_CRAFTING_INPUT, Name::CraftingInputContainer),
            (NAME_ENCHANTING_INPUT, Name::EnchantingInputContainer),
            (NAME_ENCHANTING_MATERIAL, Name::EnchantingMaterialContainer),
            (NAME_FURNACE_FUEL, Name::FurnaceFuelContainer),
            (NAME_FURNACE_INGREDIENT, Name::FurnaceIngredientContainer),
            (NAME_FURNACE_RESULT, Name::FurnaceResultContainer),
            (NAME_HORSE_EQUIP, Name::HorseEquipContainer),
            (NAME_SHULKER_BOX, Name::ShulkerBoxContainer),
            (NAME_LOOM_INPUT, Name::LoomInputContainer),
            (NAME_LOOM_DYE, Name::LoomDyeContainer),
            (NAME_LOOM_MATERIAL, Name::LoomMaterialContainer),
            (
                NAME_BLAST_FURNACE_INGREDIENT,
                Name::BlastFurnaceIngredientContainer,
            ),
            (NAME_SMOKER_INGREDIENT, Name::SmokerIngredientContainer),
            (NAME_GRINDSTONE_INPUT, Name::GrindstoneInputContainer),
            (
                NAME_GRINDSTONE_ADDITIONAL,
                Name::GrindstoneAdditionalContainer,
            ),
            (NAME_STONECUTTER_INPUT, Name::StonecutterInputContainer),
            (NAME_CARTOGRAPHY_INPUT, Name::CartographyInputContainer),
            (
                NAME_CARTOGRAPHY_ADDITIONAL,
                Name::CartographyAdditionalContainer,
            ),
            (NAME_BARREL, Name::BarrelContainer),
            (NAME_SMITHING_TEMPLATE, Name::SmithingTableTemplateContainer),
            (NAME_CRAFTER, Name::CrafterLevelEntityContainer),
            (2, Name::AnvilResultPreviewContainer),
            (5, Name::SmithingTableResultPreviewContainer),
            (14, Name::CraftingOutputPreviewContainer),
            (44, Name::LoomResultPreviewContainer),
            (52, Name::GrindstoneResultPreviewContainer),
            (54, Name::StonecutterResultPreviewContainer),
            (57, Name::CartographyResultPreviewContainer),
            (33, Name::TradeResultPreviewContainer),
        ];
        for (expected, name) in pairs {
            assert_eq!(code(name), expected);
        }
    }

    #[test]
    fn every_ui_slot_name_round_trips() {
        for slot in 0..UI_SLOT_COUNT as u8 {
            if let Some(name) = ui_slot_container_name(slot) {
                assert_eq!(ui_slot_for_name(name, u16::from(slot)), Some(slot));
            }
        }
        assert_eq!(ui_slot_for_name(0, 2), None);
    }

    #[test]
    fn named_windows_address_their_segments() {
        let (container, slot) =
            open_cell_request(WindowKind::Furnace, 1, None, None).expect("fuel cell");
        assert_eq!(
            container,
            StackRequestContainer::OpenWindow {
                name: NAME_FURNACE_FUEL,
                dynamic_id: None
            }
        );
        assert_eq!(slot, 1);
        assert!(open_cell_request(WindowKind::Furnace, 3, None, None).is_none());
        let (brew, _) = open_cell_request(WindowKind::Brewing, 3, None, None).expect("bottle");
        assert_eq!(
            brew,
            StackRequestContainer::OpenWindow {
                name: NAME_BREWING_RESULT,
                dynamic_id: None
            }
        );
        let (horse, _) = open_cell_request(WindowKind::Horse, 9, Some(4), None).expect("chest");
        assert_eq!(
            horse,
            StackRequestContainer::LevelEntity {
                dynamic_id: Some(4)
            }
        );
    }

    #[test]
    fn generic_windows_echo_barrel_and_shulker_names() {
        let (container, _) =
            open_cell_request(WindowKind::Storage, 2, None, Some(NAME_BARREL)).expect("cell");
        assert_eq!(
            container,
            StackRequestContainer::OpenWindow {
                name: NAME_BARREL,
                dynamic_id: None
            }
        );
        let (container, _) = open_cell_request(WindowKind::Hopper, 2, Some(1), None).expect("cell");
        assert_eq!(
            container,
            StackRequestContainer::LevelEntity {
                dynamic_id: Some(1)
            }
        );
    }

    #[test]
    fn window_types_classify() {
        assert_eq!(WindowKind::from_window_type(0), Some(WindowKind::Storage));
        assert_eq!(WindowKind::from_window_type(2), Some(WindowKind::Furnace));
        assert_eq!(WindowKind::from_window_type(-1), None);
        assert!(WindowKind::Anvil.is_ui_backed());
        assert!(!WindowKind::Hopper.is_ui_backed());
    }
}
