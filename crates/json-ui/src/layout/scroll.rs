//! Where a scroll view's named descendants land: the frame decides, layout re-anchors.

use super::place::{anchor_from, place_by_anchor};
use super::{LaidOut, LayoutEnv, Rect, ResolvedControl};
use crate::widgets::{Placement, ScrollFrame};

/// `child`'s rect and visibility under the innermost open scroll view, plus
/// the alpha a fading touch box gives its children.
pub(super) fn place(
    frame: Option<&mut ScrollFrame>,
    child: &ResolvedControl,
    parent: Rect,
    (rect, shown): (Rect, bool),
    env: &LayoutEnv,
) -> (Rect, bool, Option<f32>) {
    let Some(frame) = frame else {
        return (rect, shown, None);
    };
    match frame.place(child, rect, anchor_from(child)) {
        Placement::Keep => (rect, shown, None),
        Placement::Move(moved) => (moved, shown, None),
        Placement::Hide => (rect, false, None),
        Placement::Box { size, delta, fade } => {
            let at = place_by_anchor(child, parent, size, env);
            let placed = Rect::new(at.x + delta[0], at.y + delta[1], size[0], size[1]);
            frame.placed_box(placed);
            (placed, shown, fade)
        }
    }
}

/// A fading touch box dims its children, as vanilla writes their alpha.
pub(super) fn fade(laid: &mut LaidOut, alpha: Option<f32>) {
    if let Some(alpha) = alpha {
        for child in &mut laid.children {
            child.alpha *= alpha;
        }
    }
}
