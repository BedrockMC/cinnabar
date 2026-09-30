//! The bed screen (`/gameplay/bedtime`): black gradients sweeping in from the
//! top and bottom edges, the sleep status centred a quarter down a 36rem
//! column, and Leave bed (plus Open chat with other players present) 17.2%
//! above the bottom. The status and buttons fade in, and the buttons take
//! input only once the screen has settled.

use ui::UiRect;

use super::super::super::{UiPresentationError, rect};
use super::paint::Canvas;
use super::theme::{HEADER5, TEXT};
use super::widgets::{Interaction, Variant, button_face};

/// The gradient bands' strongest alpha (0.8) and where it starts (4.49%).
const BAND_ALPHA: f32 = 0.8;
const BAND_SOLID: f32 = 0.0449;
const BAND_STRIPS: usize = 24;
const BANDS_IN_MILLIS: f32 = 2_400.0;
const STATUS_DELAY_MILLIS: f32 = 200.0;
const BUTTONS_DELAY_MILLIS: f32 = 1_200.0;
const FADE_MILLIS: f32 = 1_000.0;
/// Clicks count only once the screen is interactive.
pub(crate) const INTERACTIVE_MILLIS: u64 = 1_500;

const STATUS_NIGHT: &str = "Sleeping through the night";
const STATUS_THUNDERSTORM: &str = "Sleeping through the thunderstorm";
const LEAVE_BED: &str = "Leave bed";
const OPEN_CHAT: &str = "Open chat";

/// What a press on the bed screen means.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum BedHit {
    LeaveBed,
    OpenChat,
}

/// The facet state the screen reads.
pub(crate) struct Bedtime {
    /// Milliseconds since the player lay down.
    pub(crate) elapsed: u64,
    pub(crate) remote_players: bool,
    pub(crate) thunderstorm: bool,
    pub(crate) hovered: Option<BedHit>,
    pub(crate) pressed: Option<BedHit>,
}

/// CSS `ease-in` (cubic-bezier 0.42, 0, 1, 1) at `t`.
fn ease_in(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    // Solve x(s) = t for the bezier parameter, then return y(s).
    let (mut low, mut high) = (0.0f32, 1.0f32);
    for _ in 0..24 {
        let s = (low + high) * 0.5;
        let x = 3.0 * (1.0 - s) * (1.0 - s) * s * 0.42 + 3.0 * (1.0 - s) * s * s + s * s * s;
        if x < t {
            low = s;
        } else {
            high = s;
        }
    }
    let s = (low + high) * 0.5;
    3.0 * (1.0 - s) * s * s + s * s * s
}

fn fade(elapsed: f32, delay: f32) -> f32 {
    ease_in((elapsed - delay) / FADE_MILLIS)
}

pub(super) fn draw(
    canvas: &mut Canvas<'_>,
    state: &Bedtime,
    size: [f32; 2],
) -> Result<Vec<(BedHit, UiRect)>, UiPresentationError> {
    let [width, height] = size;
    let elapsed = state.elapsed as f32;
    let band = height * ease_in(elapsed / BANDS_IN_MILLIS);
    // Each band is dark at its edge and clear at its far end.
    for strip in 0..BAND_STRIPS {
        let from = strip as f32 / BAND_STRIPS as f32;
        let to = (strip + 1) as f32 / BAND_STRIPS as f32;
        let mid = (from + to) * 0.5;
        let strength = if mid <= BAND_SOLID {
            1.0
        } else {
            1.0 - (mid - BAND_SOLID) / (1.0 - BAND_SOLID)
        };
        let colour = [0, 0, 0, (BAND_ALPHA * strength * 255.0).round() as u8];
        canvas.fill([0.0, band * from, width, band * to], colour)?;
        canvas.fill(
            [0.0, height - band * to, width, height - band * from],
            colour,
        )?;
    }
    let column = canvas.r(36.0).min(width);
    let left = (width - column) * 0.5;
    canvas.alpha = fade(elapsed, STATUS_DELAY_MILLIS);
    let status = if state.thunderstorm {
        STATUS_THUNDERSTORM
    } else {
        STATUS_NIGHT
    };
    let top = height * 0.258;
    canvas.text_centred(
        status,
        [left, top, left + column, top + canvas.r(HEADER5.line)],
        HEADER5,
        TEXT,
        false,
    )?;
    canvas.alpha = fade(elapsed, BUTTONS_DELAY_MILLIS);
    let button_height = canvas.r(4.0);
    let gap = canvas.r(0.4);
    let mut buttons = vec![(BedHit::LeaveBed, LEAVE_BED)];
    if state.remote_players {
        buttons.push((BedHit::OpenChat, OPEN_CHAT));
    }
    let block = button_height * buttons.len() as f32 + gap * (buttons.len() - 1) as f32;
    let mut y = height * (1.0 - 0.172) - block;
    let interactive = state.elapsed >= INTERACTIVE_MILLIS;
    let mut hits = Vec::new();
    for (hit, label) in buttons {
        let bounds = [left, y, left + column, y + button_height];
        let interaction = Interaction {
            hovered: interactive && state.hovered == Some(hit),
            pressed: interactive && state.pressed == Some(hit),
            focused: false,
        };
        button_face(canvas, bounds, Variant::Secondary, label, interaction, true)?;
        if interactive {
            hits.push((hit, rect(bounds[0], bounds[1], bounds[2], bounds[3])?));
        }
        y += button_height + gap;
    }
    canvas.alpha = 1.0;
    Ok(hits)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ease_in_matches_the_css_curve_at_its_ends_and_middle() {
        assert!(ease_in(0.0).abs() < 1e-4);
        assert!((ease_in(1.0) - 1.0).abs() < 1e-4);
        // cubic-bezier(0.42, 0, 1, 1) at x = 0.5 is about 0.315.
        assert!((ease_in(0.5) - 0.315).abs() < 0.01, "{}", ease_in(0.5));
    }
}
