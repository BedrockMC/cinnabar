//! Measurements shared by the size and placement passes of one layout, or kept
//! across one tree's layouts by a [`MeasureCache`].

use std::{cell::RefCell, collections::HashMap};

use super::{Axis, LayoutEnv, Rect, ResolvedControl, axis_index, grid, size, stack};
use crate::expr::{Length, Unit};

type Key = (usize, Option<u64>, Option<u64>);

#[derive(Clone, Copy, Default)]
pub(super) struct Children {
    /// Summed visible children per axis (`%c`).
    pub(super) content: [f64; 2],
    /// Largest visible child per axis (`%cm`).
    pub(super) maximum: [f64; 2],
}

type PlaceMemo = HashMap<(usize, u64, u64), Vec<(usize, Rect)>>;

thread_local! {
    static CHILDREN: RefCell<HashMap<Key, Children>> = RefCell::new(HashMap::new());
    static NATURAL: RefCell<HashMap<Key, Option<[f64; 2]>>> = RefCell::new(HashMap::new());
    /// [`sizes`] by parent address and known extent. Children sizes are pure in
    /// the subtree, `env` and that extent, but content extents re-derive them, so
    /// without this the cost is exponential in tree depth.
    static SIZES: RefCell<HashMap<Key, Vec<[f64; 2]>>> = RefCell::new(HashMap::new());
    /// Parsed `size`/`min_size`/`max_size` lengths by control address and slot.
    pub(super) static LENGTHS: RefCell<HashMap<(usize, u8), Option<Length>>> =
        RefCell::new(HashMap::new());
    /// [`placed_children`]: child indices and rects relative to the parent's
    /// origin, by parent address and size.
    static PLACED: RefCell<PlaceMemo> = RefCell::new(PlaceMemo::new());
}

/// Discard measurements before borrowing a new tree or measurement environment.
pub(super) fn reset() {
    CHILDREN.with(|memo| memo.borrow_mut().clear());
    NATURAL.with(|memo| memo.borrow_mut().clear());
    SIZES.with(|memo| memo.borrow_mut().clear());
    LENGTHS.with(|memo| memo.borrow_mut().clear());
    PLACED.with(|memo| memo.borrow_mut().clear());
}

/// Measurements of one bound tree, reused by its later layouts. Start a new one
/// whenever the tree, the root size or the measurement environment changes.
#[derive(Default)]
pub struct MeasureCache {
    children: HashMap<Key, Children>,
    natural: HashMap<Key, Option<[f64; 2]>>,
    sizes: HashMap<Key, Vec<[f64; 2]>>,
    lengths: HashMap<(usize, u8), Option<Length>>,
    placed: PlaceMemo,
    /// The root's address last layout; a moved root's entries go stale.
    root: usize,
}

impl MeasureCache {
    fn swap(&mut self) {
        CHILDREN.with(|memo| std::mem::swap(&mut *memo.borrow_mut(), &mut self.children));
        NATURAL.with(|memo| std::mem::swap(&mut *memo.borrow_mut(), &mut self.natural));
        SIZES.with(|memo| std::mem::swap(&mut *memo.borrow_mut(), &mut self.sizes));
        LENGTHS.with(|memo| std::mem::swap(&mut *memo.borrow_mut(), &mut self.lengths));
        PLACED.with(|memo| std::mem::swap(&mut *memo.borrow_mut(), &mut self.placed));
    }

    /// Make these the live memos for a layout of `root`.
    pub(super) fn enter(&mut self, root: &ResolvedControl) {
        self.swap();
        let address = std::ptr::from_ref(root).addr();
        if self.root != address {
            let stale = self.root;
            CHILDREN.with(|memo| memo.borrow_mut().retain(|key, _| key.0 != stale));
            NATURAL.with(|memo| memo.borrow_mut().retain(|key, _| key.0 != stale));
            SIZES.with(|memo| memo.borrow_mut().retain(|key, _| key.0 != stale));
            LENGTHS.with(|memo| memo.borrow_mut().retain(|key, _| key.0 != stale));
            PLACED.with(|memo| memo.borrow_mut().retain(|key, _| key.0 != stale));
            self.root = address;
        }
    }

    /// Park the live memos again after the layout.
    pub(super) fn leave(&mut self) {
        self.swap();
    }
}

/// Identify a control and its exact known `[width, height]` for the lifetime of this layout.
fn key(control: &ResolvedControl, own: [Option<f64>; 2]) -> Key {
    (
        std::ptr::from_ref(control).addr(),
        own[0].map(f64::to_bits),
        own[1].map(f64::to_bits),
    )
}

/// Measure visible children once at `own`, the control's known extent. A
/// templated grid's `%c` builds no terms, so it sums to zero.
pub(super) fn children(
    control: &ResolvedControl,
    env: &LayoutEnv,
    own: [Option<f64>; 2],
) -> Children {
    if control.children.is_empty() {
        return Children::default();
    }
    let key = key(control, own);
    if let Some(cached) = CHILDREN.with(|memo| memo.borrow().get(&key).copied()) {
        return cached;
    }
    let sizes = sizes(control, own, env);
    let mut sum = [0.0; 2];
    let mut maximum = [0.0_f64; 2];
    let resting = crate::widgets::rest_hidden_children(control);
    for (child, size) in control.children.iter().zip(&sizes) {
        if !super::visible(child) || grid::is_template_node(child) || resting.contains(&child.name)
        {
            continue;
        }
        for axis in 0..2 {
            sum[axis] += size[axis];
            maximum[axis] = maximum[axis].max(size[axis]);
        }
    }
    if grid::has_template(control) {
        sum = [0.0; 2];
    }
    let measured = Children {
        content: sum,
        maximum,
    };
    CHILDREN.with(|memo| memo.borrow_mut().insert(key, measured));
    measured
}

/// Every child's `[w, h]` under `parent` of known `extent`, by the parent's
/// kind: stack items, grid cells, or ordinary relative rules.
pub(super) fn sizes(
    parent: &ResolvedControl,
    extent: [Option<f64>; 2],
    env: &LayoutEnv,
) -> Vec<[f64; 2]> {
    let key = key(parent, extent);
    if let Some(cached) = SIZES.with(|memo| memo.borrow().get(&key).cloned()) {
        return cached;
    }
    let measured = if stack::orientation(parent).is_some() {
        stack::child_sizes(parent, extent, env)
    } else if grid::is_grid(parent) {
        grid::child_sizes(parent, extent, env)
    } else {
        relative_sizes(parent, extent, env)
    };
    SIZES.with(|memo| memo.borrow_mut().insert(key, measured.clone()));
    measured
}

/// Children sized by their own rules against `extent`.
pub(super) fn relative_sizes(
    parent: &ResolvedControl,
    extent: [Option<f64>; 2],
    env: &LayoutEnv,
) -> Vec<[f64; 2]> {
    let mut sizes = resolve_children(parent, extent, env, |_| true);
    apply_inherit(parent, &mut sizes, |_| true);
    sizes
}

/// Resolve the children `include` selects (others stay zero): those reading
/// `%sm` after the rest, against the largest resolved sibling.
pub(super) fn resolve_children(
    parent: &ResolvedControl,
    extent: [Option<f64>; 2],
    env: &LayoutEnv,
    include: impl Fn(&ResolvedControl) -> bool,
) -> Vec<[f64; 2]> {
    let mut sizes = vec![[0.0; 2]; parent.children.len()];
    let mut deferred = false;
    for (child, slot) in parent.children.iter().zip(sizes.iter_mut()) {
        if !include(child) {
            continue;
        }
        if reads_sibling_max(child) {
            deferred = true;
            continue;
        }
        *slot = size::resolve_size(child, extent, [0.0; 2], env);
    }
    if deferred {
        let siblings = sibling_maxima(parent, &sizes);
        for (child, slot) in parent.children.iter().zip(sizes.iter_mut()) {
            if include(child) && reads_sibling_max(child) {
                *slot = size::resolve_size(child, extent, siblings, env);
            }
        }
    }
    sizes
}

fn reads_sibling_max(child: &ResolvedControl) -> bool {
    [Axis::X, Axis::Y]
        .into_iter()
        .any(|axis| size::reads(child, axis, &[Unit::PercentSiblingMax]))
}

/// The largest visible child per axis among those whose size does not itself
/// read `%sm` (the client leaves those out to avoid a cycle).
pub(super) fn sibling_maxima(parent: &ResolvedControl, sizes: &[[f64; 2]]) -> [f64; 2] {
    let mut maxima = [0.0_f64; 2];
    for (child, size) in parent.children.iter().zip(sizes) {
        if !super::visible(child) || grid::is_template_node(child) {
            continue;
        }
        for axis in [Axis::X, Axis::Y] {
            let index = axis_index(axis);
            if !size::size_length(child, axis)
                .is_some_and(|length| length.uses(Unit::PercentSiblingMax))
            {
                maxima[index] = maxima[index].max(size[index]);
            }
        }
    }
    maxima
}

/// `inherit_max_sibling_width`/`height`: after solving, each inheriting child
/// (of those `include` selects) takes the largest same-axis value among itself
/// and its visible siblings, all read before any is replaced.
pub(super) fn apply_inherit(
    parent: &ResolvedControl,
    sizes: &mut [[f64; 2]],
    include: impl Fn(&ResolvedControl) -> bool,
) {
    const KEYS: [&str; 2] = ["inherit_max_sibling_width", "inherit_max_sibling_height"];
    let inherits = |child: &ResolvedControl, index: usize| {
        matches!(
            child.properties.get(KEYS[index]),
            Some(serde_json::Value::Bool(true))
        )
    };
    if !parent
        .children
        .iter()
        .any(|child| inherits(child, 0) || inherits(child, 1))
    {
        return;
    }
    let mut maxima = [0.0_f64; 2];
    for (child, size) in parent.children.iter().zip(sizes.iter()) {
        if super::visible(child) && !grid::is_template_node(child) {
            maxima = [maxima[0].max(size[0]), maxima[1].max(size[1])];
        }
    }
    for (child, size) in parent.children.iter().zip(sizes.iter_mut()) {
        for index in 0..2 {
            if include(child) && inherits(child, index) {
                size[index] = size[index].max(maxima[index]);
            }
        }
    }
}

/// Measure a label or texture once for each known width, including absent sizes.
pub(super) fn natural(
    control: &ResolvedControl,
    width: Option<f64>,
    read: impl FnOnce() -> Option<[f64; 2]>,
) -> Option<[f64; 2]> {
    let key = key(control, [width, None]);
    if let Some(cached) = NATURAL.with(|memo| memo.borrow().get(&key).copied()) {
        return cached;
    }
    let measured = read();
    NATURAL.with(|memo| memo.borrow_mut().insert(key, measured));
    measured
}

/// [`super::layout_children`], memoized by the parent and its size: children sit at the
/// same offsets from their parent wherever it is placed.
pub(super) fn placed_children<'a>(
    parent: &'a ResolvedControl,
    rect: Rect,
    env: &LayoutEnv,
) -> Vec<(&'a ResolvedControl, Rect)> {
    let key = (
        std::ptr::from_ref(parent).addr(),
        rect.w.to_bits(),
        rect.h.to_bits(),
    );
    let shift = |(index, at): &(usize, Rect)| {
        let moved = Rect::new(at.x + rect.x, at.y + rect.y, at.w, at.h);
        (&parent.children[*index], moved)
    };
    let memoized = PLACED.with(|memo| {
        let memo = memo.borrow();
        memo.get(&key)
            .map(|placed| placed.iter().map(shift).collect())
    });
    if let Some(placed) = memoized {
        return placed;
    }
    let relative: Vec<(usize, Rect)> =
        super::layout_children(parent, Rect::new(0.0, 0.0, rect.w, rect.h), env)
            .into_iter()
            .map(|(child, at)| (child_index(parent, child), at))
            .collect();
    let placed = relative.iter().map(shift).collect();
    PLACED.with(|memo| memo.borrow_mut().insert(key, relative));
    placed
}

/// A child's position among its parent's children.
pub(super) fn child_index(parent: &ResolvedControl, child: &ResolvedControl) -> usize {
    let base = parent.children.as_ptr().addr();
    let size = std::mem::size_of::<ResolvedControl>().max(1);
    (std::ptr::from_ref(child).addr() - base) / size
}
