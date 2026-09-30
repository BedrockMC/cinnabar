//! The play route's Servers tab (classic layout): the side menu in four of
//! twelve columns (Add server, featured experiences then other servers) and
//! the selected server's details in eight (10:3 banner with ping and players,
//! name with the hero Play button, then description, activities and news for
//! an experience, or edit and remove for a saved server).

use std::collections::HashMap;

use super::super::super::{IconRef, UiPresentationError};
use super::super::play_screen::play_featured;
use super::grid::{Grid, space};
use super::paint::{Bounds, Canvas};
use super::theme::{BODY, CAPTION, NEUTRAL80, NEUTRAL100, SECTION_HEADER, TEXT, TEXT_DIMMER};
use super::widgets::{Variant, button, divider, row, row_text, section_label, side_menu};
use crate::menu::{MenuAction, MenuServerCard, MenuView, PingInfo};

pub(super) fn draw(
    canvas: &mut Canvas<'_>,
    view: &MenuView,
    grid: &Grid,
    body: Bounds,
    images: &HashMap<String, IconRef>,
) -> Result<(), UiPresentationError> {
    let (menu_span, details_span) = if grid.narrow {
        ((0, 3), (3, 5))
    } else {
        ((0, 4), (4, 8))
    };
    let [menu_left, menu_right] = grid.span(menu_span.0, menu_span.1);
    side_menu(canvas, [menu_left, body[1], menu_right, body[3]])?;
    let pad = canvas.r(1.6);
    let mut y = body[1] + space(canvas, 2);
    let add_height = canvas.r(4.4);
    button(
        canvas,
        view,
        [menu_left + pad, y, menu_right - pad, y + add_height],
        Variant::Secondary,
        "Add server",
        Some(MenuAction::PlayAddServer),
    )?;
    y += add_height;
    let featured: Vec<&MenuServerCard> =
        view.featured.iter().chain(view.gatherings.iter()).collect();
    y = section_label(
        canvas,
        &format!("Featured experiences ({})", featured.len()),
        [menu_left, menu_right],
        y,
    )?;
    let item_height = canvas.r(4.8);
    let selection = selection(view, featured.len());
    for (index, server) in featured.iter().enumerate() {
        if y + item_height > body[3] {
            break;
        }
        let selected = selection == Some(Selection::Featured(index));
        let bounds = [
            menu_left + canvas.r(0.2),
            y,
            menu_right - canvas.r(0.2),
            y + item_height,
        ];
        row(
            canvas,
            view,
            bounds,
            selected,
            Some(MenuAction::SelectFeatured(index)),
        )?;
        let logo = canvas.r(4.0);
        let logo_bounds = [
            bounds[0] + pad,
            y + canvas.r(0.4),
            bounds[0] + pad + logo,
            y + canvas.r(0.4) + logo,
        ];
        match images.get(&server.image_path) {
            Some(icon) => canvas.icon_ref(*icon, logo_bounds)?,
            None => canvas.fill(logo_bounds, NEUTRAL100)?,
        }
        let text_left = logo_bounds[2] + canvas.r(0.8);
        let text_width = bounds[2] - pad - text_left;
        row_text(
            canvas,
            [text_left, y],
            text_width,
            &server.name,
            &server.caption,
        )?;
        y += item_height;
    }
    y = section_label(
        canvas,
        &format!("Other Server ({})", view.servers.len()),
        [menu_left, menu_right],
        y,
    )?;
    for (index, server) in view.servers.iter().enumerate() {
        if y + item_height > body[3] {
            break;
        }
        let bounds = [
            menu_left + canvas.r(0.2),
            y,
            menu_right - canvas.r(0.2),
            y + item_height,
        ];
        row(
            canvas,
            view,
            bounds,
            selection == Some(Selection::Saved(index)),
            Some(MenuAction::SelectSaved(index)),
        )?;
        let text_width = bounds[2] - bounds[0] - pad * 2.0;
        row_text(
            canvas,
            [bounds[0] + pad, y],
            text_width,
            &server.name,
            &server.address,
        )?;
        y += item_height;
    }

    let [left, right] = grid.span(details_span.0, details_span.1);
    let panel = [left, body[1], right, body[3]];
    match selection {
        Some(Selection::Featured(index)) => {
            details(canvas, view, featured[index], index, panel, images)
        }
        Some(Selection::Saved(index)) => saved_details(canvas, view, index, panel),
        None => Ok(()),
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Selection {
    Featured(usize),
    Saved(usize),
}

/// The picked server, else the first experience, else the first saved server.
fn selection(view: &MenuView, featured: usize) -> Option<Selection> {
    let saved = view.servers.len();
    match (view.feeds.selected_saved, view.feeds.selected_featured) {
        (Some(index), _) if index < saved => Some(Selection::Saved(index)),
        (_, Some(index)) if index < featured => Some(Selection::Featured(index)),
        _ if featured > 0 => Some(Selection::Featured(0)),
        _ if saved > 0 => Some(Selection::Saved(0)),
        _ => None,
    }
}

/// The banner's bottom strip: ping on the left, players on the right.
fn ping_strip(
    canvas: &mut Canvas<'_>,
    banner: Bounds,
    ping: Option<&PingInfo>,
) -> Result<(), UiPresentationError> {
    let overlay = [banner[0], banner[3] - canvas.r(6.0), banner[2], banner[3]];
    canvas.fill(overlay, [0, 0, 0, 179])?;
    let pad = canvas.r(2.4);
    let line_top = overlay[1] + (overlay[3] - overlay[1] - canvas.r(BODY.line)) * 0.5;
    canvas.text(
        ping_label(ping),
        [overlay[0] + pad, line_top],
        (overlay[2] - overlay[0]) * 0.5,
        BODY,
        TEXT_DIMMER,
        false,
    )?;
    if let Some(ping) = ping.filter(|ping| ping.online) {
        let players = format!("{}/{}", ping.players, ping.max_players);
        let width = canvas.measure(&players, BODY)?;
        canvas.text(
            &players,
            [overlay[2] - pad - width, line_top],
            width + 1.0,
            BODY,
            TEXT_DIMMER,
            false,
        )?;
    }
    Ok(())
}

/// A saved server: ping and players, name and address, Play, Edit and Remove.
fn saved_details(
    canvas: &mut Canvas<'_>,
    view: &MenuView,
    index: usize,
    b: Bounds,
) -> Result<(), UiPresentationError> {
    let server = &view.servers[index];
    let edge = canvas.r(0.2);
    canvas.fill(b, NEUTRAL80.fill)?;
    canvas.frame(b, 0.2, [0x1e, 0x1e, 0x1f, 255])?;
    let inner = [b[0] + edge, b[1] + edge, b[2] - edge, b[3]];
    let banner = [
        inner[0],
        inner[1],
        inner[2],
        inner[1] + (inner[2] - inner[0]) * 0.3,
    ];
    canvas.fill(banner, NEUTRAL100)?;
    ping_strip(canvas, banner, view.feeds.pings.get(&server.address))?;
    let pad = canvas.r(2.4);
    let mut y = banner[3] + space(canvas, 3);
    let play_width = canvas.r(32.0).min((inner[2] - inner[0]) * 0.45);
    let play_height = canvas.r(4.4);
    let text_width = inner[2] - inner[0] - pad * 3.0 - play_width;
    canvas.text(
        &server.name,
        [inner[0] + pad, y],
        text_width,
        BODY,
        TEXT,
        false,
    )?;
    canvas.text(
        &server.address,
        [inner[0] + pad, y + canvas.r(2.4)],
        text_width,
        CAPTION,
        TEXT_DIMMER,
        false,
    )?;
    let play_left = inner[2] - pad - play_width;
    button(
        canvas,
        view,
        [play_left, y, inner[2] - pad, y + play_height],
        Variant::Hero,
        "Play",
        Some(MenuAction::PlaySaved(index)),
    )?;
    y += play_height + space(canvas, 2);
    let half = (play_width - space(canvas, 2)) * 0.5;
    button(
        canvas,
        view,
        [play_left, y, play_left + half, y + play_height],
        Variant::Secondary,
        "Edit",
        Some(MenuAction::EditSaved(index)),
    )?;
    button(
        canvas,
        view,
        [inner[2] - pad - half, y, inner[2] - pad, y + play_height],
        Variant::Secondary,
        "Remove",
        Some(MenuAction::RemoveSavedDialog(index)),
    )
}

fn details(
    canvas: &mut Canvas<'_>,
    view: &MenuView,
    server: &MenuServerCard,
    index: usize,
    b: Bounds,
    images: &HashMap<String, IconRef>,
) -> Result<(), UiPresentationError> {
    let edge = canvas.r(0.2);
    canvas.fill(b, NEUTRAL80.fill)?;
    canvas.frame(b, 0.2, [0x1e, 0x1e, 0x1f, 255])?;
    let inner = [b[0] + edge, b[1] + edge, b[2] - edge, b[3]];
    let details = view.feeds.details.get(&server.address);
    // Banner: the first screenshot, else the logo.
    let banner = [
        inner[0],
        inner[1],
        inner[2],
        inner[1] + (inner[2] - inner[0]) * 0.3,
    ];
    let art = details
        .and_then(|details| details.screenshots.first())
        .and_then(|path| images.get(path))
        .or_else(|| images.get(&server.image_path));
    match art {
        Some(icon) => canvas.icon_ref(*icon, banner)?,
        None => canvas.fill(banner, NEUTRAL100)?,
    }
    ping_strip(canvas, banner, view.feeds.pings.get(&server.address))?;
    let pad = canvas.r(2.4);
    // Name row with the hero Play button.
    let mut y = banner[3] + space(canvas, 3);
    let play_width = canvas.r(32.0).min((inner[2] - inner[0]) * 0.45);
    let play_height = canvas.r(4.4);
    canvas.text(
        &server.name,
        [inner[0] + pad, y + canvas.r(1.2)],
        inner[2] - inner[0] - pad * 3.0 - play_width,
        BODY,
        TEXT,
        false,
    )?;
    button(
        canvas,
        view,
        [
            inner[2] - pad - play_width,
            y,
            inner[2] - pad,
            y + play_height,
        ],
        Variant::Hero,
        "Play",
        play_featured(view, index),
    )?;
    y += play_height + space(canvas, 3);
    let Some(details) = details else {
        return Ok(());
    };
    let text_width = inner[2] - inner[0] - pad * 2.0;
    for (title, text) in [
        ("Description", details.description.as_str()),
        ("News", details.news.as_str()),
    ] {
        if text.is_empty() || y > b[3] {
            continue;
        }
        divider(canvas, inner[0], inner[2], y)?;
        y += space(canvas, 3);
        y += canvas.text(
            title,
            [inner[0] + pad, y],
            text_width,
            SECTION_HEADER,
            TEXT,
            false,
        )? + space(canvas, 2);
        y += canvas.text(
            text,
            [inner[0] + pad, y],
            text_width,
            CAPTION,
            TEXT_DIMMER,
            false,
        )? + space(canvas, 3);
    }
    if !details.games.is_empty() && y < b[3] {
        divider(canvas, inner[0], inner[2], y)?;
        y += space(canvas, 3);
        y += canvas.text(
            "Activities",
            [inner[0] + pad, y],
            text_width,
            SECTION_HEADER,
            TEXT,
            false,
        )? + space(canvas, 2);
        let image = canvas.r(15.2).min(text_width * 0.3);
        for game in &details.games {
            if y + image > b[3] {
                break;
            }
            let frame = [inner[0] + pad, y, inner[0] + pad + image, y + image];
            match images.get(&game.image_path) {
                Some(icon) => canvas.icon_ref(*icon, frame)?,
                None => canvas.fill(frame, NEUTRAL100)?,
            }
            let text_left = frame[2] + space(canvas, 3);
            let width = inner[2] - pad - text_left;
            let mut line = y;
            line += canvas.text(&game.title, [text_left, line], width, BODY, TEXT, false)?;
            line += canvas.text(
                &game.subtitle,
                [text_left, line],
                width,
                CAPTION,
                TEXT_DIMMER,
                false,
            )?;
            canvas.text(
                &game.description,
                [text_left, line + space(canvas, 2)],
                width,
                CAPTION,
                TEXT_DIMMER,
                false,
            )?;
            y += image + space(canvas, 3);
        }
    }
    Ok(())
}

fn ping_label(ping: Option<&PingInfo>) -> &'static str {
    match ping {
        None => "Loading ping",
        Some(ping) if !ping.online => "Offline",
        Some(ping) if ping.ping_ms < 150 => "Low ping",
        Some(ping) if ping.ping_ms < 300 => "Medium ping",
        Some(_) => "High ping",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_first_experience_shows_until_a_server_is_picked() {
        let mut view = crate::menu::MenuRuntime::new(true, 2, "Steve".to_owned()).view();
        view.servers = vec![crate::menu::SavedServer {
            name: "Home".to_owned(),
            address: "127.0.0.1:19132".to_owned(),
            favorite: false,
            last_joined_unix: 0,
        }];
        assert_eq!(selection(&view, 0), Some(Selection::Saved(0)));
        assert_eq!(selection(&view, 2), Some(Selection::Featured(0)));
        view.feeds.select_saved(0);
        assert_eq!(selection(&view, 2), Some(Selection::Saved(0)));
        view.feeds.select(1);
        assert_eq!(selection(&view, 2), Some(Selection::Featured(1)));
    }

    #[test]
    fn ping_labels_follow_the_round_trip() {
        let pong = |ping_ms| PingInfo {
            online: true,
            players: 1,
            max_players: 2,
            ping_ms,
        };
        assert_eq!(ping_label(None), "Loading ping");
        assert_eq!(ping_label(Some(&PingInfo::default())), "Offline");
        assert_eq!(ping_label(Some(&pong(20))), "Low ping");
        assert_eq!(ping_label(Some(&pong(200))), "Medium ping");
        assert_eq!(ping_label(Some(&pong(500))), "High ping");
    }
}
