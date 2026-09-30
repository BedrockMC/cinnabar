//! Vanilla JSON-UI container screens: every inventory and station window draws
//! through the engine by default. `CINNABAR_FORM_SNAPSHOT_DIR` writes each as a
//! PNG for inspection. Needs the gitignored UI carrier; skips when absent.

use protocol::{
    ContainerIdentity, ContainerOpenEvent, InventoryAuthority, InventoryContentEvent,
    InventoryEvent, NetworkItemStack,
};

use super::engine_hud_tests::engine_presentation_with;
use super::*;
use crate::ui_runtime::presentation::inventory_pointer::InventoryCellHit;

/// A server-authoritative session with the local language table, when built.
fn session() -> UiRuntime {
    let mut runtime = UiRuntime::new(1);
    runtime.publish_inventory_authority(InventoryAuthority::Server);
    runtime.publish_local_runtime_id(1, 42).unwrap();
    let lang = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../.local/assets/compiled/vanilla-v1.mcbelang");
    if let Some(lang) = std::fs::read(lang)
        .ok()
        .and_then(|bytes| assets::RuntimeLangCatalog::decode(&bytes).ok())
    {
        runtime.set_lang_catalog(std::sync::Arc::new(lang));
    }
    runtime
}

fn opened(window_type: i8, cells: usize) -> UiRuntime {
    let mut runtime = session();
    // Chest-like windows name their content by the level-entity container.
    let generic = protocol::WindowKind::from_window_type(window_type)
        .and_then(protocol::WindowKind::open_cells)
        .is_some_and(|cells| matches!(cells, protocol::OpenCells::Generic(_)));
    let content = ContainerIdentity {
        slot_type: generic.then_some(protocol::CONTAINER_NAME_LEVEL_ENTITY),
        ..ContainerIdentity::window(7)
    };
    runtime
        .enqueue_inventory_event(
            1,
            1,
            InventoryEvent::Open(ContainerOpenEvent {
                container: ContainerIdentity::window(7),
                window_type,
                position: [0, 64, 0],
                runtime_entity_id: -1,
            }),
        )
        .unwrap();
    if cells > 0 {
        runtime
            .enqueue_inventory_event(
                1,
                2,
                InventoryEvent::Content(InventoryContentEvent {
                    container: content,
                    slots: vec![NetworkItemStack::empty(); cells].into(),
                    storage_item: NetworkItemStack::empty(),
                }),
            )
            .unwrap();
    }
    runtime.drain_pending_inventory();
    runtime
}

/// `runtime` with the open window's `ContainerSetData` properties applied.
fn with_data(mut runtime: UiRuntime, properties: &[(i32, i32)]) -> UiRuntime {
    for &(property, value) in properties {
        runtime
            .inventory_ledger_mut()
            .apply(&InventoryEvent::Data(protocol::ContainerDataEvent {
                container: ContainerIdentity::window(7),
                property,
                value,
            }));
    }
    runtime
}

/// Three offered options costing 1, 5 and 30 levels.
fn with_enchant_options(mut runtime: UiRuntime) -> UiRuntime {
    let option = |cost: u8, network_id: u32| protocol::EnchantOption {
        cost,
        name: "abc def".into(),
        network_id,
        enchants: vec![(9, cost.min(5))].into(),
    };
    runtime
        .inventory_ledger_mut()
        .apply(&InventoryEvent::EnchantOptions(
            protocol::EnchantOptionsEvent {
                options: vec![option(1, 1), option(5, 2), option(30, 3)].into(),
            },
        ));
    runtime
}

/// The creative inventory over a 300-item catalog across the four tabs, whose
/// first construction items fold into a named group, the second one unfolded.
fn creative() -> UiRuntime {
    use protocol::{CreativeCategory, CreativeContentEvent, CreativeGroup, CreativeItem};
    let mut runtime = session();
    runtime.publish_player_game_mode(protocol::PlayerGameMode::Creative);
    let categories = [
        CreativeCategory::Construction,
        CreativeCategory::Nature,
        CreativeCategory::Equipment,
        CreativeCategory::Items,
    ];
    let group = |category: CreativeCategory, name: &str| CreativeGroup {
        category,
        name: name.into(),
        icon: None,
    };
    let mut groups: Vec<CreativeGroup> = categories
        .iter()
        .map(|category| group(*category, ""))
        .collect();
    groups.push(group(
        CreativeCategory::Construction,
        "itemGroup.name.planks",
    ));
    groups.push(group(
        CreativeCategory::Construction,
        "itemGroup.name.stone",
    ));
    let items = (0..300u32)
        .map(|index| CreativeItem {
            creative_network_id: index + 1,
            stack: NetworkItemStack {
                network_id: 1 + index as i32,
                count: 1,
                ..NetworkItemStack::empty()
            },
            group: match index {
                0..20 if index % 4 == 0 => 4,
                20..40 if index % 4 == 0 => 5,
                _ => index % 4,
            },
        })
        .collect::<Vec<_>>();
    runtime
        .inventory_ledger_mut()
        .apply(&InventoryEvent::Creative(CreativeContentEvent {
            groups: groups.into(),
            items: items.into(),
            skipped: 0,
        }));
    runtime.screen_state_mut().creative_expanded.insert(5);
    runtime.toggle_inventory();
    runtime
}

fn personal() -> UiRuntime {
    let mut runtime = session();
    runtime.toggle_inventory();
    runtime
}

/// Every screen the engine draws by default, by snapshot name, with the window
/// cells its item slots must address.
fn screens() -> Vec<(&'static str, UiRuntime, Vec<InventoryCellHit>)> {
    use crate::ui_runtime::presentation::screens::Widget as W;
    use InventoryCellHit::{
        Craft, CraftOutput, CreativeSearch, CreativeTab, RecipeBook, Storage, Widget,
    };
    use protocol::*;
    let storage = |count: u8| (0..count).map(Storage).collect::<Vec<_>>();
    vec![
        (
            "inventory",
            personal(),
            vec![Craft(28), Craft(31), CraftOutput, Widget(W::BookToggle)],
        ),
        (
            "inventory_recipe_book",
            {
                let mut runtime = personal();
                runtime.screen_state_mut().book_open = true;
                runtime
            },
            vec![Widget(W::BookToggle), CreativeTab(1), CreativeSearch],
        ),
        (
            "creative",
            creative(),
            vec![
                RecipeBook(0),
                RecipeBook(20),
                CreativeTab(2),
                CreativeSearch,
                Widget(W::BookToggle),
            ],
        ),
        (
            "crafting_table",
            opened(WINDOW_TYPE_WORKBENCH, 0),
            vec![Craft(32), Craft(40), CraftOutput],
        ),
        ("chest", opened(WINDOW_TYPE_CONTAINER, 27), storage(27)),
        (
            "large_chest",
            opened(WINDOW_TYPE_CONTAINER, 54),
            storage(54),
        ),
        // Half cooked, fuel half burnt.
        (
            "furnace",
            with_data(
                opened(WINDOW_TYPE_FURNACE, 3),
                &[(0, 100), (1, 50), (2, 100)],
            ),
            storage(3),
        ),
        (
            "blast_furnace",
            opened(WINDOW_TYPE_BLAST_FURNACE, 3),
            storage(3),
        ),
        ("smoker", opened(WINDOW_TYPE_SMOKER, 3), storage(3)),
        // Half brewed, half the fuel left.
        (
            "brewing_stand",
            with_data(
                opened(WINDOW_TYPE_BREWING_STAND, 5),
                &[(0, 200), (1, 10), (2, 20)],
            ),
            storage(5),
        ),
        (
            "anvil",
            opened(WINDOW_TYPE_ANVIL, 0),
            vec![Craft(1), Craft(2), CraftOutput, Widget(W::AnvilName)],
        ),
        (
            "enchanting_table",
            with_enchant_options(opened(WINDOW_TYPE_ENCHANTMENT, 0)),
            vec![Craft(14), Craft(15)],
        ),
        (
            "grindstone",
            opened(WINDOW_TYPE_GRINDSTONE, 0),
            vec![Craft(16), Craft(17), CraftOutput],
        ),
        (
            "loom",
            opened(WINDOW_TYPE_LOOM, 0),
            vec![Craft(9), Craft(10), Craft(11), CraftOutput],
        ),
        (
            "smithing_table",
            opened(WINDOW_TYPE_SMITHING_TABLE, 0),
            vec![Craft(51), Craft(52), Craft(53), CraftOutput],
        ),
        (
            "cartography_table",
            opened(WINDOW_TYPE_CARTOGRAPHY, 0),
            vec![Craft(12), Craft(13), CraftOutput],
        ),
        (
            "stonecutter",
            opened(WINDOW_TYPE_STONECUTTER, 0),
            vec![Craft(3), CraftOutput],
        ),
        (
            "beacon",
            opened(WINDOW_TYPE_BEACON, 0),
            vec![
                Craft(27),
                Widget(W::BeaconEffect {
                    id: 1,
                    secondary: false,
                }),
                Widget(W::BeaconEffect {
                    id: 10,
                    secondary: true,
                }),
                Widget(W::BeaconUpgrade),
                Widget(W::BeaconConfirm),
            ],
        ),
        ("hopper", opened(WINDOW_TYPE_HOPPER, 5), storage(5)),
        ("dispenser", opened(WINDOW_TYPE_DISPENSER, 9), storage(9)),
        ("dropper", opened(WINDOW_TYPE_DROPPER, 9), storage(9)),
        ("crafter", opened(WINDOW_TYPE_CRAFTER, 9), storage(9)),
        ("horse", opened(WINDOW_TYPE_HORSE, 17), storage(17)),
    ]
}

// Every container screen draws through the engine, and a click on each of its
// window cells and player slots reaches that ledger cell.
#[test]
fn every_container_screen_draws_through_the_engine() {
    let only = std::env::var("CINNABAR_CONTAINER_SCREEN").ok();
    for (name, runtime, expected) in screens() {
        if only.as_deref().is_some_and(|only| only != name) {
            continue;
        }
        let Some(mut presentation) =
            engine_presentation_with(super::super::forms::pack_harness::font())
        else {
            return;
        };
        assert!(runtime.inventory_open(), "{name}");
        // Textures publish during the first builds.
        let dpi = DpiScale::new(1.0).unwrap();
        for now in [0, 500] {
            presentation.build(&runtime, now, [1280, 720], dpi).unwrap();
        }
        let input = presentation
            .build(&runtime, 5_000, [1280, 720], dpi)
            .unwrap();
        super::super::forms::snapshot::write(&input, &format!("container-{name}"));
        let frame = presentation
            .engine_container_frame()
            .unwrap_or_else(|| panic!("{name} is not engine-drawn"));
        let reached: Vec<InventoryCellHit> = frame
            .hits
            .iter()
            .filter_map(|region| {
                let center = [
                    (region.rect.x + region.rect.w / 2.0) as f32,
                    (region.rect.y + region.rect.h / 2.0) as f32,
                ];
                presentation.engine_container_hit(center)
            })
            .collect();
        let player = (0..36).map(InventoryCellHit::Player);
        for hit in expected.into_iter().chain(player) {
            assert!(reached.contains(&hit), "{name}: {hit:?} unreachable");
        }
    }
}
