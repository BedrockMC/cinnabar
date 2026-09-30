//! Measurements shared by the size and placement passes of one layout.

use std::{cell::RefCell, collections::HashMap};

use super::{Axis, LayoutEnv, ResolvedControl};

type Key = (usize, Option<u64>);

#[derive(Clone, Copy, Default)]
pub(super) struct Children {
    pub(super) content: [f64; 2],
    pub(super) maximum: [f64; 2],
}

thread_local! {
    static CHILDREN: RefCell<HashMap<Key, Children>> = RefCell::new(HashMap::new());
    static NATURAL: RefCell<HashMap<Key, Option<[f64; 2]>>> = RefCell::new(HashMap::new());
}

/// Discard measurements before borrowing a new tree or measurement environment.
pub(super) fn reset() {
    CHILDREN.with(|memo| memo.borrow_mut().clear());
    NATURAL.with(|memo| memo.borrow_mut().clear());
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
