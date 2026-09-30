//! Measurements shared by the size and placement passes of one layout, or kept
//! across one tree's layouts by a [`MeasureCache`].

use std::{cell::RefCell, collections::HashMap};

use super::{Axis, LayoutEnv, Rect, ResolvedControl};
use crate::expr::Length;

type Key = (usize, Option<u64>);

#[derive(Clone, Copy, Default)]
pub(super) struct Children {
    pub(super) content: [f64; 2],
    pub(super) maximum: [f64; 2],
}

type PlaceMemo = HashMap<(usize, u64, u64), Vec<(usize, Rect)>>;

thread_local! {
    static CHILDREN: RefCell<HashMap<Key, Children>> = RefCell::new(HashMap::new());
    static NATURAL: RefCell<HashMap<Key, Option<[f64; 2]>>> = RefCell::new(HashMap::new());
    /// [`super::intrinsic`] by control address and known parent width. Intrinsic
    /// size is pure in the subtree, `env` and that width, but content extents
    /// re-derive it, so without this the cost is exponential in tree depth.
    pub(super) static INTRINSIC: RefCell<HashMap<(usize, u64), [f64; 2]>> =
        RefCell::new(HashMap::new());
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
    INTRINSIC.with(|memo| memo.borrow_mut().clear());
    LENGTHS.with(|memo| memo.borrow_mut().clear());
    PLACED.with(|memo| memo.borrow_mut().clear());
}

/// Measurements of one bound tree, reused by its later layouts. Start a new one
/// whenever the tree, the root size or the measurement environment changes.
#[derive(Default)]
pub struct MeasureCache {
    children: HashMap<Key, Children>,
    natural: HashMap<Key, Option<[f64; 2]>>,
    intrinsic: HashMap<(usize, u64), [f64; 2]>,
    lengths: HashMap<(usize, u8), Option<Length>>,
    placed: PlaceMemo,
    /// The root's address last layout; a moved root's entries go stale.
    root: usize,
}

impl MeasureCache {
    fn swap(&mut self) {
        CHILDREN.with(|memo| std::mem::swap(&mut *memo.borrow_mut(), &mut self.children));
        NATURAL.with(|memo| std::mem::swap(&mut *memo.borrow_mut(), &mut self.natural));
        INTRINSIC.with(|memo| std::mem::swap(&mut *memo.borrow_mut(), &mut self.intrinsic));
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
            INTRINSIC.with(|memo| memo.borrow_mut().retain(|key, _| key.0 != stale));
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

/// Identify a control and its exact known width for the lifetime of this layout.
fn key(control: &ResolvedControl, width: Option<f64>) -> Key {
    (
        control as *const ResolvedControl as usize,
        width.map(f64::to_bits),
    )
}

/// Measure visible children once, retaining both content extent and largest child.
pub(super) fn children(control: &ResolvedControl, env: &LayoutEnv, width: Option<f64>) -> Children {
    if control.children.is_empty() {
        return Children::default();
    }
    let key = key(control, width);
    if let Some(cached) = CHILDREN.with(|memo| memo.borrow().get(&key).copied()) {
        return cached;
    }
    let mut sum = [0.0; 2];
    let mut maximum = [0.0_f64; 2];
    let mut count = 0;
    for child in control
        .children
        .iter()
        .filter(|child| super::visible(child))
    {
        let size = super::intrinsic(child, env, width);
        for axis in 0..2 {
            sum[axis] += size[axis];
            maximum[axis] = maximum[axis].max(size[axis]);
        }
        count += 1;
    }
    let content = if let Some(columns) = super::grid_columns(control) {
        let columns = super::fitted_columns(columns, width, maximum[0], count);
        [
            maximum[0] * columns.min(count) as f64,
            maximum[1] * count.div_ceil(columns) as f64,
        ]
    } else {
        match super::stack_axis(control) {
            Some(Axis::X) => [sum[0], maximum[1]],
            Some(Axis::Y) => [maximum[0], sum[1]],
            None => maximum,
        }
    };
    let measured = Children { content, maximum };
    CHILDREN.with(|memo| memo.borrow_mut().insert(key, measured));
    measured
}

/// Measure a label or natural-size image once for each known width, including absent sizes.
pub(super) fn natural(
    control: &ResolvedControl,
    width: Option<f64>,
    read: impl FnOnce() -> Option<[f64; 2]>,
) -> Option<[f64; 2]> {
    let key = key(control, width);
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
    let base = parent.children.as_ptr().addr();
    let size = std::mem::size_of::<ResolvedControl>().max(1);
    let relative: Vec<(usize, Rect)> =
        super::layout_children(parent, Rect::new(0.0, 0.0, rect.w, rect.h), env)
            .into_iter()
            .map(|(child, at)| ((std::ptr::from_ref(child).addr() - base) / size, at))
            .collect();
    let placed = relative.iter().map(shift).collect();
    PLACED.with(|memo| memo.borrow_mut().insert(key, relative));
    placed
}
