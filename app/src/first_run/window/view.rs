//! Lays out and draws one setup screen: logo, a translucent panel with title, body, progress
//! bar and buttons. Returns the button rectangles for hit testing.

use super::{
    super::screen::{Action, Screen},
    canvas::{Canvas, Image, Rect, Text},
};

const WHITE: [u8; 4] = [255, 255, 255, 255];
const BODY: [u8; 4] = [220, 220, 220, 255];
const PANEL: [u8; 4] = [0, 0, 0, 176];
const TRACK: [u8; 4] = [255, 255, 255, 48];
const GREEN: [u8; 4] = [60, 133, 39, 255];
const GREEN_HOVER: [u8; 4] = [74, 160, 49, 255];
const GRAY: [u8; 4] = [76, 76, 76, 235];
const GRAY_HOVER: [u8; 4] = [104, 104, 104, 245];
const MAX_BODY_LINES: usize = 10;

pub(super) struct Style<'a> {
    pub scale: f32,
    pub hovered: Option<Action>,
    pub updating: bool,
    pub logo: Option<&'a Image>,
    /// Appended to failure messages so users can find the full log.
    pub log_hint: &'a str,
}

pub(super) fn draw(
    canvas: &mut Canvas,
    text: &mut Text,
    screen: &Screen,
    style: &Style<'_>,
) -> Vec<(Action, Rect)> {
    let s = style.scale;
    let (width, height) = (canvas.width as f32, canvas.height as f32);
    let margin = 16.0 * s;
    let mut top = (height * 0.07).max(margin);
    if let Some(logo) = style.logo {
        let w = (520.0 * s).min(width - 2.0 * margin);
        let h = w * logo.height as f32 / logo.width.max(1) as f32;
        canvas.image(
            logo,
            Rect {
                x: (width - w) / 2.0,
                y: top,
                w,
                h,
            },
        );
        top += h + 24.0 * s;
    }

    let (title_px, body_px, pad, gap) = (24.0 * s, 16.0 * s, 20.0 * s, 12.0 * s);
    let panel_w = (620.0 * s).min(width - 2.0 * margin);
    let inner_w = panel_w - 2.0 * pad;
    let mut body = screen.body();
    if matches!(screen, Screen::Failed { .. }) {
        body = format!("{body}\n\nDetails: {}", style.log_hint);
    }
    let mut lines = if body.is_empty() {
        Vec::new()
    } else {
        text.wrap(&body, body_px, inner_w)
    };
    if lines.len() > MAX_BODY_LINES {
        lines.truncate(MAX_BODY_LINES);
        lines.push("…".to_owned());
    }
    let (title_lh, body_lh) = (text.line_height(title_px), text.line_height(body_px));
    let progress = screen.progress();
    let actions = screen.actions();
    let (bar_h, button_h) = (10.0 * s, 36.0 * s);
    let panel_h = pad
        + title_lh
        + if lines.is_empty() {
            0.0
        } else {
            gap + lines.len() as f32 * body_lh
        }
        + if progress.is_some() { gap + bar_h } else { 0.0 }
        + if actions.is_empty() {
            0.0
        } else {
            2.0 * gap + button_h
        }
        + pad;
    let panel = Rect {
        x: (width - panel_w) / 2.0,
        y: top + ((height - margin - top - panel_h) / 2.0).max(0.0),
        w: panel_w,
        h: panel_h,
    };
    canvas.fill(panel, PANEL);

    let (x, mut y) = (panel.x + pad, panel.y + pad);
    text.draw(canvas, x, y, title_px, WHITE, screen.title(style.updating));
    y += title_lh;
    if !lines.is_empty() {
        y += gap;
        for line in &lines {
            text.draw(canvas, x, y, body_px, BODY, line);
            y += body_lh;
        }
    }
    if let Some(fraction) = progress {
        y += gap;
        canvas.fill(
            Rect {
                x,
                y,
                w: inner_w,
                h: bar_h,
            },
            TRACK,
        );
        canvas.fill(
            Rect {
                x,
                y,
                w: inner_w * fraction.clamp(0.0, 1.0),
                h: bar_h,
            },
            GREEN,
        );
        y += bar_h;
    }
    if actions.is_empty() {
        return Vec::new();
    }
    y += 2.0 * gap;
    let widths: Vec<f32> = actions
        .iter()
        .map(|action| (text.width(screen.button_label(*action), body_px) + 32.0 * s).max(120.0 * s))
        .collect();
    let row = widths.iter().sum::<f32>() + gap * (widths.len() - 1) as f32;
    let mut bx = panel.x + (panel_w - row) / 2.0;
    let primary = screen.primary();
    let mut hits = Vec::with_capacity(actions.len());
    for (action, w) in actions.iter().zip(widths) {
        let rect = Rect {
            x: bx,
            y,
            w,
            h: button_h,
        };
        let hovered = style.hovered == Some(*action);
        let color = match (primary == Some(*action), hovered) {
            (true, false) => GREEN,
            (true, true) => GREEN_HOVER,
            (false, false) => GRAY,
            (false, true) => GRAY_HOVER,
        };
        canvas.fill(rect, color);
        let label = screen.button_label(*action);
        let label_x = bx + (w - text.width(label, body_px)) / 2.0;
        text.draw(
            canvas,
            label_x,
            y + (button_h - body_lh) / 2.0,
            body_px,
            WHITE,
            label,
        );
        hits.push((*action, rect));
        bx += w + gap;
    }
    hits
}
