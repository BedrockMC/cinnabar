//! Input over a laid-out screen: the interactive regions (buttons, toggles,
//! dropdowns, sliders, edit boxes, scroll views, modal panels) with what the
//! templates say they mean, pointer hit-testing that honours clipping and modal
//! panels, focus order, and `global` button mappings. The engine reports *which*
//! control and mapping fired; the screen's owner decides what that does.

use serde_json::Value;

use crate::emit::RectOut;
use crate::layout::{LaidOut, Rect};
use crate::widgets;

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
    /// An input panel a press routes somewhere (`button.menu_select` mapped on it).
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

/// A control's press sound (`SoundComponent`): its `sound_name`, else the first
/// `sounds` entry for a button event on the control's button (or any button).
#[derive(Clone, Debug, PartialEq)]
pub struct ControlSound {
    pub name: String,
    pub volume: f32,
    pub pitch: f32,
    /// `min_seconds_between_plays`; zero never holds a repeat back.
    pub min_seconds: f32,
}

impl ControlSound {
    fn of(control: &crate::ResolvedControl, button: Option<&str>) -> Option<Self> {
        let properties = &control.properties;
        let number = |value: Option<&Value>, fallback: f64| {
            value.and_then(Value::as_f64).unwrap_or(fallback) as f32
        };
        if let Some(name) = properties
            .get("sound_name")
            .and_then(Value::as_str)
            .filter(|name| !name.is_empty())
        {
            return Some(Self {
                name: name.to_owned(),
                volume: number(properties.get("sound_volume"), 1.0),
                pitch: number(properties.get("sound_pitch"), 1.0),
                min_seconds: 0.0,
            });
        }
        properties
            .get("sounds")?
            .as_array()?
            .iter()
            .find(|entry| {
                let event = entry.get("event_type").and_then(Value::as_str);
                let wanted = entry.get("button_name").and_then(Value::as_str);
                event == Some("button_event")
                    && wanted.is_none_or(|wanted| wanted.is_empty() || Some(wanted) == button)
            })
            .and_then(|entry| {
                Some(Self {
                    name: entry
                        .get("sound_name")
                        .and_then(Value::as_str)
                        .filter(|name| !name.is_empty())?
                        .to_owned(),
                    volume: number(entry.get("sound_volume"), 1.0),
                    pitch: number(entry.get("sound_pitch"), 1.0),
                    min_seconds: number(entry.get("min_seconds_between_plays"), 0.0),
                })
            })
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
    pub sound: Option<ControlSound>,
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
    collect(root, None, None, &mut out, &mut order);
    out.sort_by_key(|region| (region.layer, region.order));
    out
}

fn collect(
    node: &LaidOut,
    index: Option<usize>,
    collection: Option<&str>,
    out: &mut Vec<HitRegion>,
    order: &mut usize,
) {
    if !node.visible {
        return;
    }
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
    let (index, collection) = match details {
        Some((name, at)) => (Some(at as usize), Some(name)),
        None => (index, collection),
    };
    if let Some(kind) = kind_of(node) {
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
        let pressed = pressed_target(control);
        let sound = ControlSound::of(control, pressed.as_deref());
        out.push(HitRegion {
            key: node.key.clone(),
            name: control.name.clone(),
            kind,
            rect: node.rect.into(),
            clip: node.clip.into(),
            layer: node.layer,
            order: *order,
            pressed,
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
            sound,
        });
        *order += 1;
    }
    for child in &node.children {
        collect(child, index, collection, out, order);
    }
}

fn kind_of(node: &LaidOut) -> Option<HitKind> {
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
        "input_panel" if widgets::bound_bool(control, "modal") == Some(true) => HitKind::Modal,
        "input_panel" if pressed_target(control).is_some() => HitKind::Panel,
        "custom" if control.properties.contains_key("collection_index") => HitKind::Custom,
        _ => return None,
    })
}

/// The `to_button_id` of the `button.menu_select` → `pressed` mapping.
fn pressed_target(control: &crate::tree::ResolvedControl) -> Option<String> {
    mappings(control)
        .find(|(from, _, kind)| *from == Some("button.menu_select") && *kind == "pressed")
        .map(|(_, to, _)| to.to_owned())
}

/// `(from, to, mapping_type)` for each well-formed, unconditional mapping.
fn mappings(
    control: &crate::tree::ResolvedControl,
) -> impl Iterator<Item = (Option<&str>, &str, &str)> {
    let items: &[Value] = match control.properties.get("button_mappings") {
        Some(Value::Array(items)) => items,
        _ => &[],
    };
    items.iter().filter_map(|item| {
        let item = item.as_object()?;
        // A nested `ignored` keeps its substituted text (`(not false)`); fold it here.
        let ignored = match item.get("ignored") {
            Some(Value::Bool(flag)) => *flag,
            Some(Value::String(expression)) => {
                crate::predicate::eval(expression, &crate::env::Env::new()) == Some(true)
            }
            _ => false,
        };
        // A deselect-only entry (an edit box or slider letting go) applies only
        // while its control is selected, which is the caller's state, not ours.
        let flag = |key: &str| item.get(key).and_then(Value::as_bool);
        if ignored || (flag("handle_deselect") == Some(true) && flag("handle_select") != Some(true))
        {
            return None;
        }
        let to = item.get("to_button_id")?.as_str()?;
        let from = item.get("from_button_id").and_then(Value::as_str);
        let kind = item
            .get("mapping_type")
            .and_then(Value::as_str)
            .unwrap_or("global");
        Some((from, to, kind))
    })
}

/// The topmost enabled region under `point`, or `None` when nothing (or a modal
/// panel with nothing of its own there) is hit.
pub fn hit_test(regions: &[HitRegion], point: [f64; 2]) -> Option<&HitRegion> {
    let top = regions.iter().rev().find(|region| region.contains(point))?;
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
            HitKind::Modal => return None,
            _ => {}
        }
    }
    None
}

/// Focusable, enabled regions in document order (the Tab/arrow sequence).
pub fn focus_order(regions: &[HitRegion]) -> Vec<&HitRegion> {
    let mut focusable: Vec<&HitRegion> = regions
        .iter()
        .filter(|region| region.kind.focusable() && region.enabled)
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
    if let Some((_, to, _)) =
        mappings(node.control).find(|(source, _, kind)| *source == Some(from) && *kind == "global")
    {
        *found = Some(to.to_owned());
    }
    for child in &node.children {
        find_global(child, from, found);
    }
}

/// A region's rect as a layout [`Rect`].
pub fn region_rect(region: &HitRegion) -> Rect {
    Rect::new(region.rect.x, region.rect.y, region.rect.w, region.rect.h)
}
