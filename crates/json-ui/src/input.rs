//! Input over a laid-out screen: the interactive regions (buttons, toggles,
//! dropdowns, sliders, edit boxes, scroll views, modal panels) with what the
//! templates say they mean, pointer hit-testing that honours clipping and modal
//! panels, focus order, and `global` button mappings. The engine reports *which*
//! control and mapping fired; the screen's owner decides what that does.

use serde_json::Value;

use crate::emit::RectOut;
use crate::layout::{LaidOut, Rect};
use crate::widgets;

mod focus;
mod mapping;
mod navigate;

pub use focus::{
    CustomRoute, FOCUS_OVERRIDE_STOP, FocusContainer, FocusDirection, FocusMeta, NavigationMode,
};
pub use mapping::{
    InputComponent, InputMode, InputModeCondition, Mapping, MappingScope, MappingType,
};
pub use navigate::{
    FocusMove, controller_direction_claimed, default_focus, navigate, next_in_order, set_focus,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HitKind {
    Button,
    Toggle,
    /// The toggle that opens a `dropdown`'s content.
    Dropdown,
    Slider,
    EditBox,
    ScrollView,
    ScrollBox,
    ScrollTrack,
    /// A `modal` input panel: swallows the pointer for everything beneath it.
    Modal,
    /// Any other input panel; only one mapping `button.menu_select` takes the pointer.
    Panel,
    /// A `custom` renderer cell (e.g. a container item) the caller interprets.
    Custom,
}

impl HitKind {
    /// Kinds that take keyboard/gamepad focus.
    pub fn focusable(self) -> bool {
        matches!(
            self,
            HitKind::Button
                | HitKind::Toggle
                | HitKind::Dropdown
                | HitKind::Slider
                | HitKind::EditBox
        )
    }
}

/// One interactive control, in draw order.
#[derive(Clone, Debug, PartialEq)]
pub struct HitRegion {
    pub key: String,
    pub name: String,
    pub kind: HitKind,
    pub rect: RectOut,
    pub clip: RectOut,
    pub layer: i32,
    pub order: usize,
    /// Where `button.menu_select` routes when pressed (`$pressed_button_name`).
    pub pressed: Option<String>,
    /// `toggle_name` / `slider_name` / `text_box_name` / `dropdown_name`.
    pub control_name: Option<String>,
    /// The nearest enclosing factory/grid instance index.
    pub collection_index: Option<usize>,
    /// The collection of the nearest enclosing grid cell (e.g. `container_items`).
    pub collection: Option<String>,
    pub enabled: bool,
    pub checked: Option<bool>,
    pub max_length: Option<usize>,
    /// A radio toggle's `toggle_group_forced_index`, its place in the group.
    pub group_index: Option<usize>,
    /// The `custom` renderer name for [`HitKind::Custom`].
    pub renderer: Option<String>,
    pub input: InputComponent,
    pub focus: Option<FocusMeta>,
    /// Enclosing collection instances, outermost first: `(collection, index)`.
    pub collections: Vec<(String, usize)>,
    /// The control's own components (toggle, slider, edit box, sounds, …).
    pub widget: crate::component::Widget,
    /// Key of the nearest enclosing `modal` input panel (itself included).
    pub modal_root: Option<String>,
}

impl HitRegion {
    pub fn contains(&self, point: [f64; 2]) -> bool {
        let inside = |rect: &RectOut| {
            point[0] >= rect.x
                && point[0] < rect.x + rect.w
                && point[1] >= rect.y
                && point[1] < rect.y + rect.h
        };
        inside(&self.rect) && inside(&self.clip)
    }

    /// Whether focus may land here: an enabled control with an enabled focus component.
    pub fn takes_focus(&self) -> bool {
        self.enabled && self.focus.as_ref().is_some_and(|focus| focus.enabled)
    }

    /// `0..=1` position of `x` across the region, for slider drags.
    pub fn fraction_at(&self, x: f64) -> f64 {
        if self.rect.w <= 0.0 {
            return 0.0;
        }
        ((x - self.rect.x) / self.rect.w).clamp(0.0, 1.0)
    }
}

/// Every visible interactive region under `root`, ordered bottom to top.
pub fn hit_regions(root: &LaidOut) -> Vec<HitRegion> {
    let mut out = Vec::new();
    let mut order = 0usize;
    collect(
        root,
        (None, None, None),
        None,
        (&mut Vec::new(), &mut Vec::new()),
        &mut out,
        &mut order,
    );
    out.sort_by_key(|region| (region.layer, region.order));
    out
}

fn collect(
    node: &LaidOut,
    (index, collection, modal_root): (Option<usize>, Option<&str>, Option<&str>),
    panel: Option<&str>,
    (chain, containers): (&mut Vec<(String, usize)>, &mut Vec<FocusContainer>),
    out: &mut Vec<HitRegion>,
    order: &mut usize,
) {
    if !node.visible {
        return;
    }
    let entered = chain.len();
    let control = node.control;
    let index = control
        .properties
        .get("collection_index")
        .and_then(Value::as_u64)
        .map(|index| index as usize)
        .or(index);
    let collection = control
        .properties
        .get("collection_scope")
        .and_then(Value::as_str)
        .or(collection);
    // A `collection_details` binding names its own collection and index.
    let details = control
        .properties
        .get(crate::bind::COLLECTION_NAME_KEY)
        .and_then(Value::as_str)
        .zip(
            control
                .properties
                .get("#collection_index")
                .and_then(Value::as_f64),
        );
    let own_index = control
        .properties
        .get("collection_index")
        .and_then(Value::as_u64)
        .zip(
            control
                .properties
                .get("collection_scope")
                .and_then(Value::as_str)
                .or(panel),
        );
    if let Some((at, name)) = own_index {
        chain.push((name.to_owned(), at as usize));
    }
    if let Some((name, at)) = details {
        chain.push((name.to_owned(), at as usize));
    }
    let (index, collection) = match details {
        Some((name, at)) => (Some(at as usize), Some(name)),
        None => (index, collection),
    };
    let input = InputComponent::read(control);
    let modal_root = if input.modal {
        Some(node.key.as_str())
    } else {
        modal_root
    };
    if let Some(kind) = kind_of(node, &input) {
        let text = |key: &str| {
            control
                .properties
                .get(key)
                .and_then(Value::as_str)
                .filter(|value| !value.is_empty())
                .map(str::to_owned)
        };
        let control_name = match kind {
            HitKind::Toggle => text("toggle_name"),
            HitKind::Dropdown => text("dropdown_name").or_else(|| text("toggle_name")),
            HitKind::Slider => text("slider_name"),
            HitKind::EditBox => text("text_box_name"),
            _ => None,
        };
        out.push(HitRegion {
            key: node.key.clone(),
            name: control.name.clone(),
            kind,
            rect: node.rect.into(),
            clip: node.clip.into(),
            layer: node.layer,
            order: *order,
            pressed: input
                .pressed_target("button.menu_select")
                .map(str::to_owned),
            control_name,
            collection_index: index,
            collection: collection.map(str::to_owned),
            enabled: widgets::enabled(control),
            checked: matches!(kind, HitKind::Toggle | HitKind::Dropdown)
                .then(|| widgets::toggle_checked(control)),
            max_length: control
                .properties
                .get("max_length")
                .and_then(Value::as_u64)
                .map(|length| length as usize),
            group_index: control
                .properties
                .get("toggle_group_forced_index")
                .and_then(Value::as_f64)
                .filter(|index| *index >= 0.0)
                .map(|index| index as usize),
            renderer: (kind == HitKind::Custom)
                .then(|| text("renderer"))
                .flatten(),
            focus: FocusMeta::read(control, containers),
            widget: crate::component::Widget::read(node),
            collections: chain.clone(),
            input,
            modal_root: modal_root.map(str::to_owned),
        });
        *order += 1;
    }
    let panel = control
        .properties
        .get("collection_name")
        .and_then(Value::as_str)
        .or(panel);
    let container = FocusContainer::read(control, &node.key, node.rect.into());
    let opened = container.is_some();
    containers.extend(container);
    for child in &node.children {
        let stacks = (&mut *chain, &mut *containers);
        collect(
            child,
            (index, collection, modal_root),
            panel,
            stacks,
            out,
            order,
        );
    }
    if opened {
        containers.pop();
    }
    chain.truncate(entered);
}

fn kind_of(node: &LaidOut, input: &InputComponent) -> Option<HitKind> {
    let control = node.control;
    Some(match control.control_type.as_deref()? {
        "button" => HitKind::Button,
        "toggle" => HitKind::Toggle,
        "dropdown" => HitKind::Dropdown,
        "slider" => HitKind::Slider,
        "edit_box" => HitKind::EditBox,
        "scroll_view" => HitKind::ScrollView,
        "scrollbar_box" => HitKind::ScrollBox,
        "scroll_track" => HitKind::ScrollTrack,
        "input_panel" if input.modal => HitKind::Modal,
        "input_panel" => HitKind::Panel,
        "custom" if control.properties.contains_key("collection_index") => HitKind::Custom,
        _ => return None,
    })
}

/// The topmost enabled region under `point`, or `None` when nothing (or a modal
/// panel with nothing of its own there) is hit.
pub fn hit_test(regions: &[HitRegion], point: [f64; 2]) -> Option<&HitRegion> {
    let top = regions
        .iter()
        .rev()
        .filter(|region| region.kind != HitKind::Panel || region.pressed.is_some())
        .find(|region| region.contains(point))?;
    (top.kind != HitKind::Modal).then_some(top)
}

/// The scroll view whose area contains `point`, innermost first.
pub fn scroll_target(regions: &[HitRegion], point: [f64; 2]) -> Option<&HitRegion> {
    for region in regions.iter().rev() {
        if !region.contains(point) {
            continue;
        }
        match region.kind {
            HitKind::ScrollView => return Some(region),
            // An inline modal leaves the views around it scrolling.
            HitKind::Modal if !region.input.inline_modal => return None,
            _ => {}
        }
    }
    None
}

/// Regions focus can land on, in document order: enabled focus components,
/// inside the topmost modal panel when one is open.
pub fn focus_order(regions: &[HitRegion]) -> Vec<&HitRegion> {
    let modal = regions
        .iter()
        .rev()
        .find(|region| region.kind == HitKind::Modal)
        .map(|region| region.key.as_str());
    let mut focusable: Vec<&HitRegion> = regions
        .iter()
        .filter(|region| region.takes_focus())
        .filter(|region| modal.is_none_or(|root| region.modal_root.as_deref() == Some(root)))
        .collect();
    focusable.sort_by_key(|region| region.order);
    focusable
}

/// Where a `global` mapping from `from` routes, searching the visible tree
/// top-down so the innermost (last) declaration wins.
pub fn global_mapping(root: &LaidOut, from: &str) -> Option<String> {
    let mut found = None;
    find_global(root, from, &mut found);
    found
}

fn find_global(node: &LaidOut, from: &str, found: &mut Option<String>) {
    if !node.visible {
        return;
    }
    if let Some(mapping) = InputComponent::read(node.control)
        .mappings
        .into_iter()
        .find(|mapping| mapping.from == from && mapping.kind == MappingType::Global)
    {
        *found = Some(mapping.to);
    }
    for child in &node.children {
        find_global(child, from, found);
    }
}

/// A region's rect as a layout [`Rect`].
pub fn region_rect(region: &HitRegion) -> Rect {
    Rect::new(region.rect.x, region.rect.y, region.rect.w, region.rect.h)
}
