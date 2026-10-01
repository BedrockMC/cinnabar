//! Scroll views as the client's `ScrollViewComponent` lays them out: the named
//! viewport, content, track, box and bar panel are found breadth-first from the
//! view (tree order does not matter), the content shifts by the clamped offset
//! snapped to 1/8 px, the box takes `clamp(viewport / content, 0.1, 1)` of the
//! track, and the bar panel hides while the content fits.

use std::cell::RefCell;

use serde_json::Value;

use super::{LayoutEnv, Rect, ResolvedControl, measure, place};
use crate::state::{ScrollMetrics, ViewState};
use crate::widgets;

/// The named roles, in `ScrollRoles` order.
const ROLE_KEYS: [&str; 5] = [
    "scroll_view_port",
    "scroll_content",
    "scrollbar_track",
    "scrollbar_box",
    "scroll_box_and_track_panel",
];
const VIEWPORT: usize = 0;
const CONTENT: usize = 1;
const TRACK: usize = 2;
const BOX: usize = 3;
const PANEL: usize = 4;

/// Child-index paths from the view to each named role.
type Roles = [Option<Vec<usize>>; 5];

/// Role paths by view address, kept with a tree's other measurements.
pub(super) type RoleMemo = measure::Memo<usize, Roles>;

thread_local! {
    static ROLES: RefCell<RoleMemo> = RefCell::new(RoleMemo::default());
}

pub(super) fn swap(memo: &mut RoleMemo) {
    ROLES.with(|live| std::mem::swap(&mut *live.borrow_mut(), memo));
}

/// The first control named `name` breadth-first from `root`, itself included.
fn breadth_first(root: &ResolvedControl, name: &str) -> Option<Vec<usize>> {
    let mut queue = std::collections::VecDeque::from([(root, Vec::new())]);
    while let Some((node, path)) = queue.pop_front() {
        if node.name == name {
            return Some(path);
        }
        for (index, child) in node.children.iter().enumerate() {
            let mut next = path.clone();
            next.push(index);
            queue.push_back((child, next));
        }
    }
    None
}

fn roles(view: &ResolvedControl) -> Roles {
    let address = std::ptr::from_ref(view).addr();
    if let Some(found) = ROLES.with(|memo| memo.borrow().get(&address).cloned()) {
        return found;
    }
    let found: Roles = ROLE_KEYS.map(|key| {
        let name = view
            .properties
            .get(key)
            .and_then(Value::as_str)
            .filter(|name| !name.is_empty())?;
        breadth_first(view, name)
    });
    ROLES.with(|memo| memo.borrow_mut().insert(address, found.clone()));
    found
}

/// Forget the role paths of every view (a new tree).
pub(super) fn reset() {
    ROLES.with(|memo| memo.borrow_mut().clear());
}

/// The control at `path` under `view` and its unscrolled rect, with its parent's.
fn locate<'a>(
    view: &'a ResolvedControl,
    rect: Rect,
    path: &[usize],
    env: &LayoutEnv,
) -> Option<(&'a ResolvedControl, Rect, Rect)> {
    let mut node = view;
    let mut at = rect;
    let mut parent = rect;
    for &index in path {
        let child = node.children.get(index)?;
        let placed = measure::placed_children(node, at, env);
        let (_, child_rect) = placed
            .into_iter()
            .find(|(placed, _)| std::ptr::eq(*placed, child))?;
        parent = at;
        at = child_rect;
        node = child;
    }
    Some((node, at, parent))
}

fn address(control: &ResolvedControl) -> usize {
    std::ptr::from_ref(control).addr()
}

/// Whether `control` scrolls or drags horizontally (`draggable`).
fn horizontal(control: &ResolvedControl) -> bool {
    control.properties.get("draggable").and_then(Value::as_str) == Some("horizontal")
}

/// The scrollbar box's axis from its `draggable`, or `None` when it has none.
fn box_axis(control: &ResolvedControl) -> Option<usize> {
    match control.properties.get("draggable").and_then(Value::as_str) {
        Some("horizontal") => Some(0),
        Some("vertical") => Some(1),
        _ => None,
    }
}

/// A live scroll view: its solved geometry, applied as its descendants are placed.
pub(crate) struct ScrollFrame {
    pub key: String,
    content: Option<usize>,
    bar_box: Option<usize>,
    panel: Option<usize>,
    /// The content's shift from its laid-out rect.
    delta: [f64; 2],
    box_rect: Option<Rect>,
    /// Whether the bar panel hides because the content fits.
    pub panel_hidden: bool,
    pub metrics: Option<ScrollMetrics>,
}

impl ScrollFrame {
    /// Solve `view`'s scroll geometry within `rect`.
    pub fn open(
        view: &ResolvedControl,
        key: &str,
        rect: Rect,
        state: &ViewState,
        env: &LayoutEnv,
    ) -> Option<Self> {
        if view.control_type.as_deref() != Some("scroll_view") {
            return None;
        }
        let paths = roles(view);
        let find = |role: usize| {
            paths[role]
                .as_ref()
                .and_then(|path| locate(view, rect, path, env))
        };
        let content = find(CONTENT);
        let viewport = find(VIEWPORT);
        let track = find(TRACK);
        let bar_box = find(BOX);
        let panel = find(PANEL);
        let mut frame = Self {
            key: key.to_owned(),
            content: content.map(|(control, _, _)| address(control)),
            bar_box: bar_box.map(|(control, _, _)| address(control)),
            panel: panel.map(|(control, _, _)| address(control)),
            delta: [0.0; 2],
            box_rect: None,
            panel_hidden: false,
            metrics: None,
        };
        let (Some((content, content_rect, _)), Some((_, viewport_rect, _))) = (content, viewport)
        else {
            return Some(frame);
        };
        let axis = usize::from(!horizontal(content));
        let extent = |rect: Rect| [rect.w, rect.h];
        let content_size = extent(content_rect);
        let viewport_size = extent(viewport_rect);
        let ext = [
            (content_size[0] - viewport_size[0]).max(0.0),
            (content_size[1] - viewport_size[1]).max(0.0),
        ];
        let max = ext[axis];
        let jump = widgets::bound_bool(view, "jump_to_bottom_on_update") == Some(true)
            && state.scroll_max.get(key) != Some(&max);
        let force = widgets::bound_bool(view, "#force_scroll_to_end") == Some(true);
        let requested = if jump || force {
            max
        } else {
            state.scroll_offset(key)
        };
        let position = requested.clamp(0.0, max);
        let shown = (position * 8.0).trunc() * 0.125;
        let from = place::anchor_from(content);
        let mut delta = [0.0; 2];
        delta[axis] = -shown;
        if from[0] == 1.0 {
            delta[0] += ext[0];
        }
        if from[1] == 1.0 {
            delta[1] += ext[1];
        }
        frame.delta = delta;
        let fits = content_size[axis] <= 0.0 || viewport_size[axis] / content_size[axis] >= 1.0;
        let always = widgets::bound_bool(view, "scrollbar_always_visible") == Some(true);
        let thumb_axis = bar_box.and_then(|(control, _, _)| box_axis(control));
        frame.panel_hidden = thumb_axis.is_some() && fits && !always;
        let mut metrics = ScrollMetrics {
            offset: position,
            content: content_size[axis],
            viewport: viewport_size[axis],
            viewport_top: [viewport_rect.x, viewport_rect.y][axis],
            port: Some([
                viewport_rect.x,
                viewport_rect.y,
                viewport_rect.w,
                viewport_rect.h,
            ]),
            track: track.map(|(_, rect, _)| [rect.x, rect.y, rect.w, rect.h]),
            thumb: None,
            speed: widgets::bound_number(view, "scroll_speed").unwrap_or(1.0),
            horizontal: axis == 0,
            scrolled_to_end: max <= position,
            hit_bottom: fits || (content_size[1] - viewport_size[1]) <= position,
            bar_visible: !frame.panel_hidden,
            always_handle: widgets::bound_bool(view, "always_handle_scrolling") == Some(true),
            touch_mode: widgets::bound_bool(view, "touch_mode") == Some(true),
            gesture_control: widgets::bound_bool(view, "#gesture_control_enabled")
                .or_else(|| widgets::bound_bool(view, "gesture_control_enabled"))
                .unwrap_or(false),
            scroll_when_fits: widgets::bound_bool(view, "allow_scroll_even_when_content_fits")
                .unwrap_or(true),
            track_clicks: routes_to(
                track.map(|(control, _, _)| control),
                view,
                "scrollbar_track_button",
            ),
            touch_drags: routes_to(Some(view), view, "scrollbar_touch_button"),
        };
        if let (Some(thumb_axis), Some((control, box_rect, box_parent)), Some((_, track_rect, _))) =
            (thumb_axis, bar_box, track)
        {
            let track_size = extent(track_rect);
            let ratio = if content_size[thumb_axis] > 0.0 {
                (viewport_size[thumb_axis] / content_size[thumb_axis]).clamp(0.1, 1.0)
            } else {
                1.0
            };
            let mut size = extent(box_rect);
            size[thumb_axis] = (ratio * track_size[thumb_axis]).ceil();
            let base = place::place_by_anchor(control, box_parent, size, [0.0; 2], env);
            let travel = track_size[thumb_axis] - size[thumb_axis];
            let span = ext[thumb_axis];
            let fraction = if span > 0.5 { shown / span } else { 1.0 };
            let mut at = [base.x, base.y];
            at[thumb_axis] += travel * fraction;
            let placed = Rect::new(at[0], at[1], size[0], size[1]);
            metrics.thumb = Some([placed.x, placed.y, placed.w, placed.h]);
            frame.box_rect = Some(placed);
        }
        frame.metrics = Some(metrics);
        Some(frame)
    }

    /// The control this view shifts, sizes or hides, and how.
    pub fn adjust(&self, child: &ResolvedControl, rect: Rect) -> Adjusted {
        let at = address(child);
        if self.content == Some(at) && self.metrics.is_some() {
            return Adjusted::Moved(Rect::new(
                rect.x + self.delta[0],
                rect.y + self.delta[1],
                rect.w,
                rect.h,
            ));
        }
        if self.bar_box == Some(at)
            && let Some(placed) = self.box_rect
        {
            return Adjusted::Moved(placed);
        }
        Adjusted::Kept
    }

    /// The bar panel's address, when the view names one.
    pub fn panel_address(&self) -> Option<usize> {
        self.panel
    }
}

/// Whether `control`'s `button.menu_select` press routes to the button `view`
/// names under `key`.
fn routes_to(control: Option<&ResolvedControl>, view: &ResolvedControl, key: &str) -> bool {
    let named = view.properties.get(key).and_then(Value::as_str);
    named.is_some_and(|named| {
        control.and_then(crate::input::pressed_target).as_deref() == Some(named)
    })
}

pub(crate) enum Adjusted {
    Kept,
    Moved(Rect),
}
