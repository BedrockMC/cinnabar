//! Measurements shared by the size and placement passes of one layout, or kept
//! across one tree's layouts by a [`MeasureCache`].

use std::{
    cell::RefCell,
    collections::{HashMap, HashSet},
};

use super::{Axis, LayoutEnv, Rect, ResolvedControl, axis_index, grid, size, stack};
use crate::expr::{Length, Unit};

type Key = (usize, Option<u64>, Option<u64>);

/// Every child's `[w, h]`, shared so memo hits do not copy.
pub(super) type Sizes = std::sync::Arc<[[f64; 2]]>;

/// Memo maps keyed by control addresses and small integers, hashed cheaply.
pub(super) type Memo<K, V> = HashMap<K, V, std::hash::BuildHasherDefault<AddressHasher>>;

/// A multiply-rotate hasher for address keys; SipHash dominated layout time.
#[derive(Default)]
pub(super) struct AddressHasher(u64);

impl std::hash::Hasher for AddressHasher {
    fn write(&mut self, bytes: &[u8]) {
        for byte in bytes {
            self.write_u64(u64::from(*byte));
        }
    }

    fn write_u64(&mut self, value: u64) {
        self.0 = (self.0.rotate_left(5) ^ value).wrapping_mul(0x517c_c1b7_2722_0a95);
    }

    fn write_usize(&mut self, value: usize) {
        self.write_u64(value as u64);
    }

    fn write_u8(&mut self, value: u8) {
        self.write_u64(u64::from(value));
    }

    fn finish(&self) -> u64 {
        self.0
    }
}

#[derive(Clone, Copy, Default)]
pub(super) struct Children {
    /// Summed visible children per axis (`%c`).
    pub(super) content: [f64; 2],
    /// Largest visible child per axis (`%cm`).
    pub(super) maximum: [f64; 2],
}

type PlaceMemo = Memo<(usize, u64, u64), Vec<(usize, Rect)>>;

thread_local! {
    static CHILDREN: RefCell<Memo<Key, Children>> = RefCell::new(Memo::default());
    static NATURAL: RefCell<Memo<Key, Option<[f64; 2]>>> = RefCell::new(Memo::default());
    /// [`sizes`] by parent address and known extent. Children sizes are pure in
    /// the subtree, `env` and that extent, but content extents re-derive them, so
    /// without this the cost is exponential in tree depth.
    static SIZES: RefCell<Memo<Key, Sizes>> = RefCell::new(Memo::default());
    /// Parsed `size`/`min_size`/`max_size` lengths by control address and slot.
    pub(super) static LENGTHS: RefCell<Memo<(usize, u8), Option<Length>>> =
        RefCell::new(Memo::default());
    /// Per-control rule flags derived from the lengths.
    pub(super) static FLAGS: RefCell<Memo<usize, size::Flags>> = RefCell::new(Memo::default());
    /// [`placed_children`]: child indices and rects relative to the parent's
    /// origin, by parent address and size.
    static PLACED: RefCell<PlaceMemo> = RefCell::new(PlaceMemo::default());
    /// Scroll bar panels hidden while their content fits, by address.
    static SUPPRESSED: RefCell<HashSet<usize>> = RefCell::new(HashSet::new());
    /// This layout's clock (seconds), for `size` animations.
    static CLOCK: std::cell::Cell<Option<f64>> = const { std::cell::Cell::new(None) };
    /// Whether a `size` animation was mid-flight this layout.
    static ANIMATING: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Start a layout at `clock`; returns nothing until [`animating`] is read.
pub(super) fn start_clock(clock: Option<f64>) {
    CLOCK.with(|cell| cell.set(clock));
    ANIMATING.with(|cell| cell.set(false));
}

pub(super) fn clock() -> Option<f64> {
    CLOCK.with(std::cell::Cell::get)
}

pub(super) fn note_animating() {
    ANIMATING.with(|cell| cell.set(true));
}

/// Whether a `size` animation ran this layout.
pub(super) fn animating() -> bool {
    ANIMATING.with(std::cell::Cell::get)
}

/// Discard measurements before borrowing a new tree or measurement environment.
pub(super) fn reset() {
    CHILDREN.with(|memo| memo.borrow_mut().clear());
    NATURAL.with(|memo| memo.borrow_mut().clear());
    SIZES.with(|memo| memo.borrow_mut().clear());
    LENGTHS.with(|memo| memo.borrow_mut().clear());
    FLAGS.with(|memo| memo.borrow_mut().clear());
    PLACED.with(|memo| memo.borrow_mut().clear());
    SUPPRESSED.with(|set| set.borrow_mut().clear());
    super::scroll::reset();
}

/// Whether a scroll view hides `control`, its bar panel.
pub(super) fn suppressed(control: &ResolvedControl) -> bool {
    SUPPRESSED.with(|set| {
        let set = set.borrow();
        !set.is_empty() && set.contains(&std::ptr::from_ref(control).addr())
    })
}

/// Hide or show a scroll view's bar panel; when that changes, the measurements
/// that saw the old visibility are dropped. Returns whether it changed.
pub(super) fn suppress(panel: usize, hidden: bool) -> bool {
    let changed = SUPPRESSED.with(|set| {
        let mut set = set.borrow_mut();
        if hidden {
            set.insert(panel)
        } else {
            set.remove(&panel)
        }
    });
    if changed {
        CHILDREN.with(|memo| memo.borrow_mut().clear());
        SIZES.with(|memo| memo.borrow_mut().clear());
        PLACED.with(|memo| memo.borrow_mut().clear());
    }
    changed
}

/// Measurements of one bound tree, reused by its later layouts. Start a new one
/// whenever the tree, the root size or the measurement environment changes.
#[derive(Default)]
pub struct MeasureCache {
    children: Memo<Key, Children>,
    natural: Memo<Key, Option<[f64; 2]>>,
    sizes: Memo<Key, Sizes>,
    lengths: Memo<(usize, u8), Option<Length>>,
    flags: Memo<usize, size::Flags>,
    placed: PlaceMemo,
    suppressed: HashSet<usize>,
    roles: super::scroll::RoleMemo,
    /// The root's address last layout; a moved root's entries go stale.
    root: usize,
}

impl MeasureCache {
    fn swap(&mut self) {
        CHILDREN.with(|memo| std::mem::swap(&mut *memo.borrow_mut(), &mut self.children));
        NATURAL.with(|memo| std::mem::swap(&mut *memo.borrow_mut(), &mut self.natural));
        SIZES.with(|memo| std::mem::swap(&mut *memo.borrow_mut(), &mut self.sizes));
        LENGTHS.with(|memo| std::mem::swap(&mut *memo.borrow_mut(), &mut self.lengths));
        FLAGS.with(|memo| std::mem::swap(&mut *memo.borrow_mut(), &mut self.flags));
        PLACED.with(|memo| std::mem::swap(&mut *memo.borrow_mut(), &mut self.placed));
        SUPPRESSED.with(|set| std::mem::swap(&mut *set.borrow_mut(), &mut self.suppressed));
        super::scroll::swap(&mut self.roles);
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
            FLAGS.with(|memo| memo.borrow_mut().retain(|key, _| *key != stale));
            PLACED.with(|memo| memo.borrow_mut().retain(|key, _| key.0 != stale));
            SUPPRESSED.with(|set| set.borrow_mut().clear());
            super::scroll::reset();
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
    let templated = grid::is_grid(control);
    for (child, size) in control.children.iter().zip(sizes.iter()) {
        if !super::visible(child)
            || (templated && grid::is_template_node(child))
            || resting.contains(&child.name)
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
pub(super) fn sizes(parent: &ResolvedControl, extent: [Option<f64>; 2], env: &LayoutEnv) -> Sizes {
    let key = key(parent, extent);
    if let Some(cached) = SIZES.with(|memo| memo.borrow().get(&key).cloned()) {
        return cached;
    }
    let measured: Sizes = if stack::orientation(parent).is_some() {
        stack::child_sizes(parent, extent, env).into()
    } else if grid::is_grid(parent) {
        grid::child_sizes(parent, extent, env).into()
    } else {
        relative_sizes(parent, extent, env).into()
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
    size::flags(child).reads_sibling_max
}

/// The largest visible child per axis among those whose size does not itself
/// read `%sm` (the client leaves those out to avoid a cycle).
pub(super) fn sibling_maxima(parent: &ResolvedControl, sizes: &[[f64; 2]]) -> [f64; 2] {
    let mut maxima = [0.0_f64; 2];
    let templated = grid::is_grid(parent);
    for (child, size) in parent.children.iter().zip(sizes) {
        if !super::visible(child) || (templated && grid::is_template_node(child)) {
            continue;
        }
        for axis in [Axis::X, Axis::Y] {
            let index = axis_index(axis);
            if !size::with_size(child, axis, |length| {
                length.is_some_and(|length| length.uses(Unit::PercentSiblingMax))
            }) {
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
    let inherits = |child: &ResolvedControl, index: usize| size::flags(child).inherits[index];
    if !parent
        .children
        .iter()
        .any(|child| inherits(child, 0) || inherits(child, 1))
    {
        return;
    }
    let templated = grid::is_grid(parent);
    let mut maxima = [0.0_f64; 2];
    for (child, size) in parent.children.iter().zip(sizes.iter()) {
        if super::visible(child) && !(templated && grid::is_template_node(child)) {
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
