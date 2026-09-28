//! Container screens through the engine. The inventory ledger currently opens
//! storage windows (27 or 54 slots), which draw from the vanilla chest screens;
//! the personal inventory and workbench stay on the Java-styled path until their
//! screen globals (recipe book, creative tabs, paper doll) are bound. Slot data
//! feeds the vanilla collections (`container_items`, `inventory_items`,
//! `hotbar_items`); item icons reach `inventory_item_renderer` through an index
//! into this frame's icon table.

use json_ui::{
    CollectionItem, Context, DataSource, Draw, DrawNode, RectOut, Scalar, TextAlign, ViewState,
    hit_test,
};
use protocol::NetworkItemStack;
use serde_json::Value;
use ui::UiNode;

use super::super::{HudFrame, IconRef, TextMetrics, UiPresentationError, UiPresentationRuntime};
use super::engine;
use crate::ui_runtime::{
    UiRuntime,
    forms::EngineFrame,
    presentation::inventory_pointer::{InventoryCellHit, InventoryScreen},
};

impl UiPresentationRuntime {
    /// Draw an open storage window through the engine. `Ok(false)` leaves the
    /// screen to the Java-styled path.
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
        let InventoryScreen::Storage(slots) = InventoryScreen::of(runtime.inventory_ledger())
        else {
            return Ok(false);
        };
        let Some(renderer) = self.form_presentation.engine.as_deref() else {
            return Ok(false);
        };
        let (reference, title_key) = if slots == 54 {
            ("chest.large_chest_screen", "container.chestDouble")
        } else {
            ("chest.small_chest_screen", "container.chest")
        };
        let title = runtime
            .translation(title_key)
            .map_or_else(|| "Chest".to_owned(), |title| title.to_string());
        let context = Context::desktop()
            .with_var("container_title", Value::String(title))
            .with_flag("localize_title", false);
        let mut icons = Vec::new();
        let data = container_data(runtime, &self.hud_frame, slots, &mut icons);
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
            [width, height],
            &mut icons,
        );
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
        match renderer.render_screen(reference, &data, &context, &view, &icons, inputs, out) {
            Ok(Some(frame)) => {
                self.form_presentation.container = Some(frame);
                Ok(true)
            }
            // A screen the engine cannot draw hands storage back to the Java path.
            Ok(None) | Err(_) => {
                nodes.truncate(rollback.0);
                *next = rollback.1;
                self.hud_frame.engine_containers = false;
                Ok(false)
            }
        }
    }

    /// The container frame the engine drew last build, if any.
    pub(crate) fn engine_container_frame(&self) -> Option<&EngineFrame> {
        self.form_presentation.container.as_ref()
    }
}

/// The ledger cell under a container-screen point (virtual pixels equal GUI pixels).
pub(crate) fn engine_cell_hit(frame: &EngineFrame, gui: [f32; 2]) -> Option<InventoryCellHit> {
    let region = hit_test(&frame.hits, [f64::from(gui[0]), f64::from(gui[1])])?;
    let index = u8::try_from(region.collection_index?).ok()?;
    Some(match region.collection.as_deref()? {
        "container_items" => InventoryCellHit::Storage(index),
        "inventory_items" => InventoryCellHit::Player(index.checked_add(9)?),
        "hotbar_items" => InventoryCellHit::Player(index),
        "armor_items" => InventoryCellHit::Armor(index),
        "offhand_items" => InventoryCellHit::Offhand,
        "crafting_output_items" => InventoryCellHit::CraftOutput,
        _ => return None,
    })
}

/// Whether a point lies on the engine-drawn container's `root_panel`.
pub(crate) fn engine_panel_contains(frame: &EngineFrame, gui: [f32; 2]) -> bool {
    let (x, y) = (f64::from(gui[0]), f64::from(gui[1]));
    frame
        .panel
        .is_some_and(|[px, py, pw, ph]| x >= px && x < px + pw && y >= py && y < py + ph)
}

fn container_data(
    runtime: &UiRuntime,
    frame: &HudFrame,
    slots: usize,
    icons: &mut Vec<IconRef>,
) -> DataSource {
    let ledger = runtime.inventory_ledger();
    let mut cell = |stack: Option<&NetworkItemStack>, icon: Option<IconRef>| {
        let mut item = CollectionItem::default();
        if let (Some(_), Some(icon)) = (stack, icon) {
            icons.push(icon);
            item = item.with("#item_renderer_data", Scalar::Num((icons.len() - 1) as f64));
        }
        let count = stack.map_or(0, |stack| stack.count);
        item.with(
            "#inventory_stack_count",
            Scalar::Text(if count > 1 {
                count.to_string()
            } else {
                String::new()
            }),
        )
        .with("#item_durability_visible", Scalar::Bool(false))
    };
    let player_icon = |index: usize| frame.inventory_icons.0.get(index).copied().flatten();
    let mut data = DataSource::new();
    let storage = (0..slots)
        .map(|index| {
            cell(
                ledger.storage_stack(index as u8),
                frame.storage_icons.0.get(index).copied().flatten(),
            )
        })
        .collect();
    data.set_collection("container_items", storage);
    let inventory = (9..36)
        .map(|index| cell(ledger.displayed_stack(index as u8), player_icon(index)))
        .collect();
    data.set_collection("inventory_items", inventory);
    let hotbar = (0..9)
        .map(|index| cell(ledger.displayed_stack(index as u8), player_icon(index)))
        .collect();
    data.set_collection("hotbar_items", hotbar);
    data
}

/// The held stack drawn under the pointer, above the screen.
fn held_stack(
    stack: Option<&NetworkItemStack>,
    icon: Option<IconRef>,
    pointer: Option<[f32; 2]>,
    content: [f32; 2],
    icons: &mut Vec<IconRef>,
) -> Vec<DrawNode> {
    let (Some(stack), Some(icon), Some(point)) = (stack, icon, pointer) else {
        return Vec::new();
    };
    icons.push(icon);
    let screen = RectOut {
        x: 0.0,
        y: 0.0,
        w: f64::from(content[0]),
        h: f64::from(content[1]),
    };
    let (x, y) = (f64::from(point[0]) - 8.0, f64::from(point[1]) - 8.0);
    let node = |dest: RectOut, draw: Draw| DrawNode {
        name: "held_item".to_owned(),
        key: String::new(),
        dest,
        clip: screen,
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
