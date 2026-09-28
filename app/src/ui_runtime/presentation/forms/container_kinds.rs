//! Which vanilla screen each Bedrock container window type opens, and how its
//! flat storage slots split across that screen's item collections (in slot
//! order). Window type numbers are the protocol's container-type codes; the
//! per-collection slot counts follow the templates' grids. Types the inventory
//! ledger does not yet admit stay dormant here until it does.

/// One container window's screen and slot collections.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct ContainerKind {
    pub(crate) screen: &'static str,
    pub(crate) title_key: &'static str,
    /// `(collection, slots)` in storage-slot order.
    pub(crate) collections: &'static [(&'static str, usize)],
}

const SMALL_CHEST: ContainerKind = ContainerKind {
    screen: "chest.small_chest_screen",
    title_key: "container.chest",
    collections: &[("container_items", 27)],
};
const LARGE_CHEST: ContainerKind = ContainerKind {
    screen: "chest.large_chest_screen",
    title_key: "container.chestDouble",
    collections: &[("container_items", 54)],
};

/// The generic window type (`0`) serves chests, barrels, shulkers and ender
/// chests; its slot count picks the small or large chest layout.
pub(crate) fn container_kind(window_type: i8, slots: usize) -> Option<&'static ContainerKind> {
    Some(match window_type {
        0 if slots == 54 => &LARGE_CHEST,
        0 => &SMALL_CHEST,
        2 => &ContainerKind {
            screen: "furnace.furnace_screen",
            title_key: "container.furnace",
            collections: &[
                ("furnace_ingredient_items", 1),
                ("furnace_fuel_items", 1),
                ("furnace_output_items", 1),
            ],
        },
        3 => &ContainerKind {
            screen: "enchanting.enchanting_screen",
            title_key: "container.enchant",
            collections: &[("enchanting_input_items", 1), ("enchanting_lapis_items", 1)],
        },
        4 => &ContainerKind {
            screen: "brewing_stand.brewing_stand_screen",
            title_key: "container.brewing",
            collections: &[
                ("brewing_input_item", 1),
                ("brewing_result_items", 3),
                ("brewing_fuel_item", 1),
            ],
        },
        5 => &ContainerKind {
            screen: "anvil.anvil_screen",
            title_key: "container.repair",
            collections: &[
                ("anvil_input_items", 1),
                ("anvil_material_items", 1),
                ("anvil_result_items", 1),
            ],
        },
        6 => &ContainerKind {
            screen: "redstone.dispenser_screen",
            title_key: "container.dispenser",
            collections: &[("container_items", 9)],
        },
        7 => &ContainerKind {
            screen: "redstone.dropper_screen",
            title_key: "container.dropper",
            collections: &[("container_items", 9)],
        },
        8 => &ContainerKind {
            screen: "redstone.hopper_screen",
            title_key: "container.hopper",
            collections: &[("container_items", 5)],
        },
        12 => &ContainerKind {
            screen: "horse.horse_screen",
            title_key: "container.horse",
            collections: &[("horse_equip_items", 2), ("container_items", 15)],
        },
        13 => &ContainerKind {
            screen: "beacon.beacon_screen",
            title_key: "container.beacon",
            collections: &[("beacon_payment_items", 1)],
        },
        24 => &ContainerKind {
            screen: "loom.loom_screen",
            title_key: "container.loom",
            collections: &[
                ("loom_input_items", 1),
                ("loom_dye_items", 1),
                ("loom_material_items", 1),
                ("loom_result_items", 1),
            ],
        },
        26 => &ContainerKind {
            screen: "grindstone.grindstone_screen",
            title_key: "container.grindstone_title",
            collections: &[
                ("grindstone_input_items", 1),
                ("grindstone_additional_items", 1),
                ("grindstone_result_items", 1),
            ],
        },
        27 => &ContainerKind {
            screen: "blast_furnace.blast_furnace_screen",
            title_key: "container.blast_furnace",
            collections: &[
                ("furnace_ingredient_items", 1),
                ("furnace_fuel_items", 1),
                ("furnace_output_items", 1),
            ],
        },
        28 => &ContainerKind {
            screen: "smoker.smoker_screen",
            title_key: "container.smoker",
            collections: &[
                ("furnace_ingredient_items", 1),
                ("furnace_fuel_items", 1),
                ("furnace_output_items", 1),
            ],
        },
        29 => &ContainerKind {
            screen: "stonecutter.stonecutter_screen",
            title_key: "container.stonecutter",
            collections: &[
                ("stonecutter_input_items", 1),
                ("stonecutter_result_items", 1),
            ],
        },
        30 => &ContainerKind {
            screen: "cartography.cartography_screen",
            title_key: "container.cartography_table",
            collections: &[
                ("cartography_input_items", 1),
                ("cartography_additional_items", 1),
                ("cartography_result_items", 1),
            ],
        },
        33 => &ContainerKind {
            screen: "smithing_table.smithing_table_screen",
            title_key: "container.smithing_table",
            collections: &[
                ("smithing_table_input_items", 1),
                ("smithing_table_material_items", 1),
                ("smithing_table_result_items", 1),
            ],
        },
        _ => return None,
    })
}

impl ContainerKind {
    /// The flat storage slot of `index` within `collection`, if it belongs here.
    pub(crate) fn storage_slot(&self, collection: &str, index: usize) -> Option<usize> {
        let mut start = 0;
        for (name, slots) in self.collections {
            if *name == collection {
                return (index < *slots).then_some(start + index);
            }
            start += slots;
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generic_windows_pick_the_chest_by_slot_count() {
        assert_eq!(
            container_kind(0, 27).unwrap().screen,
            "chest.small_chest_screen"
        );
        assert_eq!(
            container_kind(0, 54).unwrap().screen,
            "chest.large_chest_screen"
        );
        assert!(container_kind(99, 1).is_none());
    }

    #[test]
    fn collections_split_the_flat_storage_slots_in_order() {
        let furnace = container_kind(2, 3).unwrap();
        assert_eq!(furnace.storage_slot("furnace_fuel_items", 0), Some(1));
        assert_eq!(furnace.storage_slot("furnace_output_items", 0), Some(2));
        assert_eq!(furnace.storage_slot("furnace_output_items", 1), None);
        assert_eq!(furnace.storage_slot("inventory_items", 0), None);
    }

    #[test]
    fn every_kind_is_an_allow_listed_engine_screen() {
        for window_type in -1..40 {
            if let Some(kind) = container_kind(window_type, 27) {
                assert!(json_ui::is_engine_screen(kind.screen), "{}", kind.screen);
            }
        }
    }
}
