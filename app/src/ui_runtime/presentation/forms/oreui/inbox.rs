//! The inbox route (`/inbox`): header, the category side menu in three of
//! twelve columns and the message cards in seven (1 | 3 | 7 | 1), split into
//! Recent (unread) and History; an empty-state panel when there is nothing.

use super::super::super::UiPresentationError;
use super::grid::{Grid, space};
use super::paint::Canvas;
use super::theme::{
    BADGE, BODY, CAPTION, NEUTRAL, SECTION_HEADER, TEXT, TEXT_DIMMER, TEXT_DIMMEST,
};
use super::widgets::{divider, header, panel, row, screen_overlay};
use crate::menu::{InboxItem, MenuAction, MenuScreen, MenuView};

pub(super) fn draw(
    canvas: &mut Canvas<'_>,
    view: &MenuView,
    size: [f32; 2],
) -> Result<(), UiPresentationError> {
    let [width, height] = size;
    screen_overlay(canvas, size)?;
    let top = header(
        canvas,
        view,
        "Inbox",
        width,
        Some(MenuAction::Navigate(MenuScreen::Home)),
    )? + space(canvas, 2);
    let bottom = height - space(canvas, 2);
    let grid = Grid::new(canvas.r(1.0), width);
    let (menu_span, list_span) = if grid.narrow {
        ((0, 2), (2, 6))
    } else {
        ((1, 3), (4, 7))
    };
    let inbox = &view.feeds.home.inbox;

    // Side menu: "All" then each category with its unread count.
    let [menu_left, menu_right] = grid.span(menu_span.0, menu_span.1);
    panel(canvas, [menu_left, top, menu_right, bottom])?;
    let mut categories: Vec<(&str, usize)> =
        vec![("All", inbox.iter().filter(|item| item.unread).count())];
    for item in inbox {
        let name = item.category.as_str();
        if name.is_empty() {
            continue;
        }
        match categories
            .iter_mut()
            .find(|(existing, _)| *existing == name)
        {
            Some((_, unread)) => *unread += usize::from(item.unread),
            None => categories.push((name, usize::from(item.unread))),
        }
    }
    let item_height = canvas.r(4.8);
    let pad = space(canvas, 4);
    let mut y = top + space(canvas, 4);
    for (index, (name, unread)) in categories.iter().enumerate() {
        let bounds = [
            menu_left + canvas.r(0.2),
            y,
            menu_right - canvas.r(0.2),
            y + item_height,
        ];
        if index == 0 {
            canvas.fill(bounds, NEUTRAL.fill)?;
        }
        let text_top = y + (item_height - canvas.r(BODY.line)) * 0.5;
        canvas.text(
            name,
            [bounds[0] + pad, text_top],
            bounds[2] - bounds[0] - pad * 3.0,
            BODY,
            TEXT,
            false,
        )?;
        if *unread > 0 {
            let count = unread.to_string();
            let badge_width = canvas.measure(&count, CAPTION)? + canvas.r(1.2);
            let badge = [
                bounds[2] - pad - badge_width,
                text_top,
                bounds[2] - pad,
                text_top + canvas.r(2.0),
            ];
            canvas.fill(badge, BADGE)?;
            canvas.text_centred(&count, badge, CAPTION, TEXT, false)?;
        }
        y += item_height;
    }

    let [list_left, list_right] = grid.span(list_span.0, list_span.1);
    if inbox.is_empty() {
        let card = [list_left, top, list_right, top + canvas.r(16.0)];
        panel(canvas, card)?;
        canvas.text_centred(
            "No messages",
            [card[0], card[1], card[2], card[1] + canvas.r(8.0)],
            SECTION_HEADER,
            TEXT,
            false,
        )?;
        canvas.text_centred(
            "Messages from Minecraft will appear here.",
            [card[0], card[1] + canvas.r(6.0), card[2], card[3]],
            CAPTION,
            TEXT_DIMMER,
            false,
        )?;
        return Ok(());
    }
    let mut y = top;
    for (title, unread) in [("Recent", true), ("History", false)] {
        let items: Vec<&InboxItem> = inbox.iter().filter(|item| item.unread == unread).collect();
        if items.is_empty() {
            continue;
        }
        y += canvas.text(
            title,
            [list_left, y],
            list_right - list_left,
            SECTION_HEADER,
            TEXT,
            false,
        )? + space(canvas, 2);
        for item in items {
            if y > bottom {
                return Ok(());
            }
            y = card(canvas, view, item, [list_left, list_right], y)? + space(canvas, 1);
        }
        divider(canvas, list_left, list_right, y)?;
        y += space(canvas, 2);
    }
    Ok(())
}

/// One detailed message card; returns its bottom edge.
fn card(
    canvas: &mut Canvas<'_>,
    view: &MenuView,
    item: &InboxItem,
    span: [f32; 2],
    top: f32,
) -> Result<f32, UiPresentationError> {
    let pad = space(canvas, 4);
    let inner = span[1] - span[0] - pad * 2.0;
    let height = canvas.r(2.0) + canvas.r(2.0) * 2.0 + pad * 2.0;
    let bounds = [span[0], top, span[1], top + height];
    row(canvas, view, bounds, false, None)?;
    let mut y = top + pad;
    let title = if item.header.is_empty() {
        "Message"
    } else {
        item.header.as_str()
    };
    y += canvas.text(title, [span[0] + pad, y], inner, BODY, TEXT, false)?;
    canvas.text(
        &item.body,
        [span[0] + pad, y + space(canvas, 1)],
        inner,
        CAPTION,
        TEXT_DIMMEST,
        false,
    )?;
    Ok(bounds[3])
}
