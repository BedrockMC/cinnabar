//! The play route's Realms tab: the side menu in four of twelve columns (Add
//! or join, then your and joined Realms) and a Realm's details in eight (10:3
//! image, name and tags, the hero Play button).

use super::super::super::UiPresentationError;
use super::grid::{Grid, space};
use super::paint::{Bounds, Canvas};
use super::theme::{
    BODY, CAPTION, NEUTRAL80, NEUTRAL100, SECONDARY_BUTTON, TEXT, TEXT_DARK, TEXT_DIMMER,
};
use super::widgets::{Variant, button, row, section_label, side_menu, tag};
use crate::menu::{MenuAction, MenuRealmCard, MenuView};

/// Owner and invited tag fill.
const PRIMARY_TINT: [u8; 4] = [0x6c, 0xc3, 0x49, 255];

pub(super) fn draw(
    canvas: &mut Canvas<'_>,
    view: &MenuView,
    grid: &Grid,
    body: Bounds,
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
        "Add/join Realm",
        None,
    )?;
    y += add_height;
    let item_height = canvas.r(4.8);
    let mut first = None;
    for (label, member) in [("Your Realms", false), ("Joined Realms", true)] {
        let realms: Vec<(usize, &MenuRealmCard)> = view
            .realms
            .iter()
            .enumerate()
            .filter(|(_, realm)| realm.member == member)
            .collect();
        y = section_label(
            canvas,
            &format!("{label} ({})", realms.len()),
            [menu_left, menu_right],
            y,
        )?;
        for (index, realm) in realms {
            first.get_or_insert(index);
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
                first == Some(index),
                Some(MenuAction::PlayRealm(index)),
            )?;
            let text_width = bounds[2] - bounds[0] - pad * 2.0;
            canvas.text(
                &realm.name,
                [bounds[0] + pad, y + canvas.r(0.2)],
                text_width,
                BODY,
                TEXT,
                false,
            )?;
            let detail = if member {
                realm.owner.as_str()
            } else {
                realm.state.as_str()
            };
            canvas.text(
                detail,
                [bounds[0] + pad, y + canvas.r(2.4)],
                text_width,
                CAPTION,
                TEXT_DIMMER,
                false,
            )?;
            y += item_height;
        }
    }
    let [left, right] = grid.span(details_span.0, details_span.1);
    let panel = [left, body[1], right, body[3]];
    canvas.fill(panel, NEUTRAL80.fill)?;
    canvas.frame(panel, 0.2, [0x1e, 0x1e, 0x1f, 255])?;
    let Some(index) = first else {
        canvas.text_centred(
            "No Realms yet",
            [left, body[1], right, body[1] + canvas.r(10.0)],
            SECONDARY_BUTTON,
            TEXT,
            false,
        )?;
        return Ok(());
    };
    let realm = &view.realms[index];
    let edge = canvas.r(0.2);
    let image = [
        left + edge,
        body[1] + edge,
        right - edge,
        body[1] + edge + (right - left) * 0.3,
    ];
    canvas.fill(image, NEUTRAL100)?;
    let players = format!("{}/{}", realm.online_players, realm.max_players);
    let overlay = [image[0], image[3] - canvas.r(4.0), image[2], image[3]];
    canvas.fill(overlay, [0, 0, 0, 179])?;
    canvas.text(
        &players,
        [overlay[0] + canvas.r(1.6), overlay[1] + canvas.r(1.0)],
        overlay[2] - overlay[0],
        BODY,
        TEXT_DIMMER,
        false,
    )?;
    let pad = canvas.r(2.4);
    let mut y = image[3] + space(canvas, 3);
    y += canvas.text(
        &realm.name,
        [left + pad, y],
        right - left - pad * 2.0,
        BODY,
        TEXT,
        false,
    )? + space(canvas, 1);
    let owner_tag = if realm.member { "Invited" } else { "Owner" };
    tag(canvas, owner_tag, [left + pad, y], PRIMARY_TINT, TEXT_DARK)?;
    y += canvas.r(2.0) + space(canvas, 3);
    let play_width = canvas.r(32.0).min((right - left) * 0.5);
    button(
        canvas,
        view,
        [left + pad, y, left + pad + play_width, y + canvas.r(4.4)],
        Variant::Hero,
        "Play",
        Some(MenuAction::PlayRealm(index)),
    )
}
