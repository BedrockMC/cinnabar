//! Container screens through the engine, behind the container-routing setting
//! (off keeps the Java-styled screens). The personal inventory and workbench
//! draw from the vanilla crafting screens in their classic survival layout; a
//! storage window picks its screen from its container type. Slot data feeds the
//! vanilla collections; item icons reach `inventory_item_renderer` through an
//! index into this frame's icon table, and `#hover_text` feeds the tooltips.

use json_ui::{
    CollectionItem, Context, DataSource, Draw, DrawNode, RectOut, Scalar, TextAlign, ViewState,
    hit_test,
};
use protocol::NetworkItemStack;
use serde_json::Value;
use ui::UiNode;

use super::super::{HudFrame, IconRef, TextMetrics, UiPresentationError, UiPresentationRuntime};
use super::container_kinds::{ContainerKind, container_kind};
use super::engine;
use crate::ui_runtime::{
    UiRuntime,
    forms::EngineFrame,
    inventory_ledger::InventoryTarget,
    presentation::inventory_pointer::{InventoryCellHit, InventoryScreen},
};

/// First UI inventory slot of the personal 2x2 and the workbench 3x3 grids.
const PERSONAL_CRAFT_SLOT: u8 = 28;
const WORKBENCH_CRAFT_SLOT: u8 = 32;
/// A clip wide enough to never cut the held stack (virtual px).
const UNCLIPPED: f64 = 1.0e5;

/// Which screen the engine drew, for mapping its cells back to ledger targets.
#[derive(Clone, Copy, Debug)]
pub(super) enum ScreenLayout {
    Personal,
    Workbench,
    Storage(&'static ContainerKind),
}

impl ScreenLayout {
    fn of(runtime: &UiRuntime) -> Option<Self> {
        let ledger = runtime.inventory_ledger();
        Some(match InventoryScreen::of_runtime(runtime) {
            InventoryScreen::Personal => Self::Personal,
            InventoryScreen::Workbench => Self::Workbench,
            InventoryScreen::Storage(slots) => {
                Self::Storage(container_kind(ledger.storage_window_type()?, slots)?)
            }
            // Other windows and the creative catalog keep the Java-styled screens.
            InventoryScreen::Window(..) | InventoryScreen::Creative => return None,
        })
    }

    fn screen(self) -> (&'static str, &'static str) {
        match self {
            Self::Personal => ("crafting.inventory_screen", "container.crafting"),
            Self::Workbench => ("crafting.crafting_screen", "container.crafting"),
            Self::Storage(kind) => (kind.screen, kind.title_key),
        }
    }

    fn craft_slot(self) -> u8 {
        match self {
            Self::Workbench => WORKBENCH_CRAFT_SLOT,
            _ => PERSONAL_CRAFT_SLOT,
        }
    }
}

impl UiPresentationRuntime {
    /// Draw the open inventory or container through the engine. `Ok(false)`
    /// leaves the screen to the Java-styled path.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn append_engine_container(
        &mut self,
        runtime: &UiRuntime,
        previous: Option<&EngineFrame>,
        nodes: &mut Vec<UiNode>,
        next: &mut u32,
        metrics: TextMetrics,
        width: f32,
        height: f32,
    ) -> Result<bool, UiPresentationError> {
        if !self.hud_frame.engine_containers || !runtime.inventory_open() {
            return Ok(false);
        }
        let Some(layout) = ScreenLayout::of(runtime) else {
            return Ok(false);
        };
        let Some(renderer) = self.form_presentation.engine.as_deref() else {
            return Ok(false);
        };
        let (reference, title_key) = layout.screen();
        let title = runtime
            .translation(title_key)
            .map_or_else(|| title_key.to_owned(), |title| title.to_string());
        let context = Context::desktop()
            .with_var("container_title", Value::String(title.clone()))
            .with_flag("localize_title", false);
        let mut icons = Vec::new();
        let data = screen_data(runtime, &self.hud_frame, layout, &title, &mut icons);
        let pointer = runtime.inventory_pointer_gui();
        let view = ViewState {
            hovered: previous
                .zip(pointer)
                .and_then(|(frame, point)| {
                    hit_test(&frame.hits, [f64::from(point[0]), f64::from(point[1])])
                })
                .map(|region| region.key.clone()),
            ..ViewState::default()
        };
        let overlay = held_stack(
            runtime.inventory_ledger().cursor_stack(),
            self.hud_frame.cursor_icon,
            pointer,
            &mut icons,
        );
        let art = engine::ScreenArt {
            icons: &icons,
            preview: self.hud_frame.player_preview,
            pointer,
            images: None,
        };
        let translate = |key: &str| runtime.translation(key);
        let rollback = (nodes.len(), *next);
        let inputs = engine::EngineInputs {
            layouts: &mut self.layouts,
            font: &self.font,
            metrics,
            solid_page: self.solid_texture_page,
            safe_area: self.safe_area,
            content: [width, height],
            translate: &translate,
        };
        let out = engine::EngineOutput {
            nodes: &mut *nodes,
            next: &mut *next,
            overlay: &overlay,
        };
        match renderer.render_screen(reference, &data, &context, &view, art, inputs, out) {
            Ok(Some(frame)) => {
                self.form_presentation.container = Some((frame, layout));
                Ok(true)
            }
            // A screen the engine cannot draw hands containers back to the Java path.
            Ok(None) | Err(_) => {
                nodes.truncate(rollback.0);
                *next = rollback.1;
                self.hud_frame.engine_containers = false;
                Ok(false)
            }
        }
    }

    /// The container-routing setting: on draws container screens through the
    /// engine (when the carrier loaded), off keeps the Java-styled screens.
    pub(crate) fn set_engine_containers(&mut self, enabled: bool) {
        self.hud_frame.engine_containers = enabled && self.form_presentation.engine.is_some();
    }

    /// The container frame the engine drew last build, if any.
    pub(crate) fn engine_container_frame(&self) -> Option<&EngineFrame> {
        self.form_presentation
            .container
            .as_ref()
            .map(|(frame, _)| frame)
    }

    /// The ledger cell under a container-screen point (virtual px equal GUI px).
    pub(crate) fn engine_container_hit(&self, gui: [f32; 2]) -> Option<InventoryCellHit> {
        let (frame, layout) = self.form_presentation.container.as_ref()?;
        let region = hit_test(&frame.hits, [f64::from(gui[0]), f64::from(gui[1])])?;
        let index = region.collection_index?;
        let small = u8::try_from(index).ok()?;
        Some(match region.collection.as_deref()? {
            "inventory_items" => InventoryCellHit::Player(small.checked_add(9)?),
            "hotbar_items" => InventoryCellHit::Player(small),
            "armor_items" => InventoryCellHit::Armor(small),
            "offhand_items" => InventoryCellHit::Offhand,
            "crafting_input_items" => {
                InventoryCellHit::Craft(layout.craft_slot().checked_add(small)?)
            }
            "crafting_output_items" => InventoryCellHit::CraftOutput,
            collection => match layout {
                ScreenLayout::Storage(kind) => InventoryCellHit::Storage(
                    u8::try_from(kind.storage_slot(collection, index)?).ok()?,
                ),
                _ => return None,
            },
        })
    }
}

/// Whether a point lies on the engine-drawn container's `root_panel`.
pub(crate) fn engine_panel_contains(frame: &EngineFrame, gui: [f32; 2]) -> bool {
    let (x, y) = (f64::from(gui[0]), f64::from(gui[1]));
    frame
        .panel
        .is_some_and(|[px, py, pw, ph]| x >= px && x < px + pw && y >= py && y < py + ph)
}

/// Builds one screen's collections, pushing each drawn icon into `icons`.
struct Cells<'a> {
    frame: &'a HudFrame,
    icons: &'a mut Vec<IconRef>,
}

impl Cells<'_> {
    fn cell(
        &mut self,
        stack: Option<&NetworkItemStack>,
        icon: Option<IconRef>,
        durability: Option<f32>,
    ) -> CollectionItem {
        let mut item = CollectionItem::default();
        if let (Some(_), Some(icon)) = (stack, icon) {
            self.icons.push(icon);
            item = item.with(
                "#item_renderer_data",
                Scalar::Num((self.icons.len() - 1) as f64),
            );
        }
        let count = stack.map_or(0, |stack| stack.count);
        let name = stack
            .and_then(|stack| {
                self.frame
                    .item_names
                    .get(&(stack.network_id, stack.metadata))
            })
            .map_or_else(String::new, |name| name.to_string());
        item.with(
            "#inventory_stack_count",
            Scalar::Text(if count > 1 {
                count.to_string()
            } else {
                String::new()
            }),
        )
        .with("#hover_text", Scalar::Text(name))
        .with(
            "#item_durability_visible",
            Scalar::Bool(durability.is_some()),
        )
        .with("#item_durability_total_amount", Scalar::Num(1000.0))
        .with(
            "#item_durability_current_amount",
            Scalar::Num(f64::from(durability.unwrap_or(1.0)) * 1000.0),
        )
    }
}

fn screen_data(
    runtime: &UiRuntime,
    frame: &HudFrame,
    layout: ScreenLayout,
    title: &str,
    icons: &mut Vec<IconRef>,
) -> DataSource {
    let ledger = runtime.inventory_ledger();
    let mut data = DataSource::new();
    let mut cells = Cells { frame, icons };
    let player_icon = |index: usize| frame.inventory_icons.0.get(index).copied().flatten();
    let inventory = (9..36)
        .map(|index| {
            cells.cell(
                ledger.displayed_stack(index as u8),
                player_icon(index),
                None,
            )
        })
        .collect();
    data.set_collection("inventory_items", inventory);
    let hotbar = (0..9)
        .map(|index| {
            cells.cell(
                ledger.displayed_stack(index as u8),
                player_icon(index),
                frame.hotbar_durability[index],
            )
        })
        .collect();
    data.set_collection("hotbar_items", hotbar);
    match layout {
        ScreenLayout::Personal | ScreenLayout::Workbench => {
            let width = if matches!(layout, ScreenLayout::Workbench) {
                3
            } else {
                2
            };
            let first = layout.craft_slot();
            let grid = (0..width * width)
                .map(|index| {
                    let target = InventoryTarget::Craft(first + index as u8);
                    cells.cell(
                        ledger.target_stack(target),
                        frame.crafting.icons.get(index).copied().flatten(),
                        None,
                    )
                })
                .collect();
            data.set_collection("crafting_input_items", grid);
            let output = match &frame.crafting.output {
                Some((icon, stack)) => cells.cell(Some(stack), *icon, None),
                None => cells.cell(None, None, None),
            };
            data.set_collection("crafting_output_items", vec![output]);
            let armor = (0..4u8)
                .map(|slot| {
                    let stack = ledger.target_stack(InventoryTarget::Armor(slot));
                    cells
                        .cell(stack, frame.armor_icons[usize::from(slot)], None)
                        .with("#empty_armor_image_visible", Scalar::Bool(stack.is_none()))
                })
                .collect();
            data.set_collection("armor_items", armor);
            let offhand = ledger.target_stack(InventoryTarget::Offhand);
            let offhand = cells
                .cell(offhand, frame.offhand_icon, frame.offhand_durability)
                .with(
                    "#empty_offhand_image_visible",
                    Scalar::Bool(offhand.is_none()),
                );
            data.set_collection("offhand_items", vec![offhand]);
        }
        ScreenLayout::Storage(kind) => {
            let mut slot = 0usize;
            for (collection, count) in kind.collections {
                let items = (0..*count)
                    .map(|offset| {
                        let index = slot + offset;
                        cells.cell(
                            ledger.storage_stack(index as u8),
                            frame.storage_icons.0.get(index).copied().flatten(),
                            None,
                        )
                    })
                    .collect();
                data.set_collection(*collection, items);
                slot += count;
            }
        }
    }
    survival_globals(&mut data, title);
    data
}

/// The classic survival layout on desktop: no recipe book, no creative tabs.
fn survival_globals(data: &mut DataSource, title: &str) {
    for (name, value) in [
        ("#is_survival_layout", true),
        ("#is_recipe_book_layout", false),
        ("#is_creative_mode", false),
        ("#is_creative_layout", false),
        ("#is_creative_layout_button_visible", false),
        ("#is_left_tab_inventory", true),
        ("#needs_crafting_table", false),
        ("#show_persistent_bundle_hover_text", true),
        ("#gamepad_helper_visible", false),
        ("#filtering_enabled", false),
    ] {
        data.set_global(name, Scalar::Bool(value));
    }
    data.set_global("#crafting_label_text", Scalar::Text(title.to_owned()));
}

/// The held stack drawn under the pointer, above the screen.
fn held_stack(
    stack: Option<&NetworkItemStack>,
    icon: Option<IconRef>,
    pointer: Option<[f32; 2]>,
    icons: &mut Vec<IconRef>,
) -> Vec<DrawNode> {
    let (Some(stack), Some(icon), Some(point)) = (stack, icon, pointer) else {
        return Vec::new();
    };
    icons.push(icon);
    let clip = RectOut {
        x: -UNCLIPPED,
        y: -UNCLIPPED,
        w: UNCLIPPED * 2.0,
        h: UNCLIPPED * 2.0,
    };
    let (x, y) = (f64::from(point[0]) - 8.0, f64::from(point[1]) - 8.0);
    let node = |dest: RectOut, draw: Draw| DrawNode {
        name: "held_item".to_owned(),
        key: String::new(),
        dest,
        clip,
        layer: i32::MAX,
        alpha: 1.0,
        draw,
    };
    let mut nodes = vec![node(
        RectOut {
            x,
            y,
            w: 16.0,
            h: 16.0,
        },
        Draw::Custom {
            renderer: "inventory_item_renderer".to_owned(),
            data: [(
                "#item_renderer_data".to_owned(),
                Value::from((icons.len() - 1) as u64),
            )]
            .into_iter()
            .collect(),
        },
    )];
    if stack.count > 1 {
        nodes.push(node(
            RectOut {
                x,
                y: y + 8.0,
                w: 17.0,
                h: 9.0,
            },
            Draw::Text {
                text: stack.count.to_string(),
                color: [255; 4],
                shadow: true,
                align: TextAlign::Right,
                scale: 1.0,
            },
        ));
    }
    nodes
}
