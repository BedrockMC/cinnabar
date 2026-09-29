//! The profile screen as the 26.30 OreUI profile route lays it out: a header
//! with the back button, then a twelve-column body with the player card in
//! four columns (featured-screenshot banner, large gamerpic, name and
//! presence, primary action) and the Overview/Stats tabs in eight (friends,
//! followers and gamerscore rows). Colours come from the bundle's menus theme;
//! exact sizes need a screenshot check.

use ui::{UiNode, UiNodeId, UiRect, UiVisual};

use super::super::{FONT_DESIGN_PIXEL_TEXELS, IconRef, TextMetrics, UiPresentationError, rect};
use crate::menu::{MenuAction, MenuScreen, MenuView, auth::AuthState};

const SCREEN: [u8; 4] = [0x1e, 0x1e, 0x1f, 235];
const HEADER: [u8; 4] = [0x31, 0x32, 0x33, 255];
const CARD: [u8; 4] = [0x24, 0x24, 0x25, 255];
const BANNER: [u8; 4] = [0x1e, 0x1e, 0x1f, 255];
const ROW: [u8; 4] = [0x31, 0x32, 0x33, 255];
const TAB_SELECTED: [u8; 4] = [0x3c, 0x85, 0x27, 255];
const PRIMARY: [u8; 4] = [0x3c, 0x85, 0x27, 255];
const PRIMARY_HOVER: [u8; 4] = [0x6c, 0xc3, 0x49, 255];
const TEXT: [u8; 4] = [0xff, 0xff, 0xff, 255];
const MUTED: [u8; 4] = [0xb1, 0xb2, 0xb5, 255];

/// Header height and body spacing, in UI design pixels.
const HEADER_HEIGHT: f32 = 26.0;
const GUTTER: f32 = 8.0;
const ROW_HEIGHT: f32 = 22.0;
const GAMERPIC: f32 = 48.0;

/// What the profile screen draws with, borrowed from the presentation runtime.
pub(super) struct ProfileInputs<'a> {
    pub(super) layouts: &'a mut ui::TextLayoutCache,
    pub(super) font: &'a assets::RuntimeFontCatalog,
    pub(super) metrics: TextMetrics,
    pub(super) solid_page: u16,
    pub(super) portrait: Option<IconRef>,
    pub(super) size: [f32; 2],
}

struct Painter<'a, 'b> {
    inputs: ProfileInputs<'a>,
    nodes: &'b mut Vec<UiNode>,
    next: &'b mut u32,
}

impl Painter<'_, '_> {
    fn node(&mut self, bounds: [f32; 4], visual: UiVisual) -> Result<UiRect, UiPresentationError> {
        let area = rect(bounds[0], bounds[1], bounds[2], bounds[3])?;
        self.nodes
            .push(UiNode::new(UiNodeId::new(*self.next), None, area).with_visual(visual));
        *self.next = self.next.saturating_add(1);
        Ok(area)
    }

    fn fill(&mut self, bounds: [f32; 4], color: [u8; 4]) -> Result<UiRect, UiPresentationError> {
        let texture_page = self.inputs.solid_page;
        self.node(
            bounds,
            UiVisual::Solid {
                texture_page,
                color,
            },
        )
    }

    fn text(
        &mut self,
        value: &str,
        at: [f32; 2],
        width: f32,
        color: [u8; 4],
    ) -> Result<(), UiPresentationError> {
        let metrics = self.inputs.metrics;
        let layout = self
            .inputs
            .layouts
            .layout(metrics.request(value, (width.max(1.0) * 64.0) as u32, self.inputs.font))
            .map_err(UiPresentationError::Text)?;
        let height = layout.size_64()[1] as f32 / 64.0;
        self.node(
            [
                at[0],
                at[1],
                at[0] + width.max(1.0),
                at[1] + height.max(1.0),
            ],
            UiVisual::Text {
                layout,
                color,
                shadow: metrics.shadow(),
            },
        )?;
        Ok(())
    }
}

/// Draws the profile screen and returns its hit targets.
pub(super) fn append(
    view: &MenuView,
    inputs: ProfileInputs<'_>,
    nodes: &mut Vec<UiNode>,
    next: &mut u32,
) -> Result<Vec<(MenuAction, UiRect)>, UiPresentationError> {
    let px = inputs.metrics.scale.get() * FONT_DESIGN_PIXEL_TEXELS as f32;
    let [width, height] = inputs.size;
    let mut paint = Painter {
        inputs,
        nodes,
        next,
    };
    let mut hits = Vec::new();
    let u = |value: f32| value * px;
    paint.fill([0.0, 0.0, width, height], SCREEN)?;
    // Header: back button left, title centred.
    paint.fill([0.0, 0.0, width, u(HEADER_HEIGHT)], HEADER)?;
    let back = paint.fill([u(4.0), u(4.0), u(22.0), u(22.0)], ROW)?;
    paint.text("<", [u(9.0), u(8.0)], u(10.0), TEXT)?;
    hits.push((MenuAction::Navigate(MenuScreen::Home), back));
    let title = "Profile";
    paint.text(title, [width * 0.5 - u(20.0), u(8.0)], u(80.0), TEXT)?;

    // Twelve columns with a one-column margin each side on wide screens.
    let column = width / 12.0;
    let top = u(HEADER_HEIGHT + GUTTER);
    let card = [
        column,
        top,
        column * 5.0 - u(GUTTER * 0.5),
        height - u(GUTTER),
    ];
    paint.fill(card, CARD)?;
    let banner_bottom = card[1] + (card[2] - card[0]) * 9.0 / 16.0;
    paint.fill([card[0], card[1], card[2], banner_bottom], BANNER)?;
    let pic = [
        card[0] + u(GUTTER),
        banner_bottom - u(GAMERPIC * 0.5),
        card[0] + u(GUTTER + GAMERPIC),
        banner_bottom + u(GAMERPIC * 0.5),
    ];
    match paint.inputs.portrait {
        Some(icon) => {
            paint.node(
                pic,
                UiVisual::Sprite {
                    texture_page: icon.page,
                    uv: icon.uv,
                    color: [255; 4],
                },
            )?;
        }
        None => {
            paint.fill(pic, ROW)?;
        }
    }
    let profile = &view.feeds.profile;
    let name = if profile.gamertag.is_empty() {
        view.display_name.as_str()
    } else {
        profile.gamertag.as_str()
    };
    let text_left = card[0] + u(GUTTER);
    let text_width = card[2] - card[0] - u(GUTTER * 2.0);
    let mut y = pic[3] + u(GUTTER);
    paint.text(name, [text_left, y], text_width, TEXT)?;
    y += u(12.0);
    let status = if !profile.real_name.is_empty() {
        profile.real_name.as_str()
    } else if !profile.presence.is_empty() {
        profile.presence.as_str()
    } else {
        match view.auth_state {
            AuthState::Authenticated => "Online",
            _ => "Offline",
        }
    };
    paint.text(status, [text_left, y], text_width, MUTED)?;
    y += u(16.0);
    // The primary action: sign in when signed out, else the dressing room.
    let signed_in = view.auth_state == AuthState::Authenticated;
    let action = if signed_in {
        None
    } else {
        Some(MenuAction::StartSignIn)
    };
    let label = if signed_in {
        "Dressing Room"
    } else {
        "Sign In"
    };
    let hovered = action.is_some() && view.hovered == action;
    let button = paint.fill(
        [text_left, y, text_left + text_width, y + u(20.0)],
        if hovered { PRIMARY_HOVER } else { PRIMARY },
    )?;
    paint.text(
        label,
        [text_left + u(6.0), y + u(6.0)],
        text_width - u(12.0),
        TEXT,
    )?;
    if let Some(action) = action {
        hits.push((action, button));
    }

    // Tabs and the overview rows.
    let right = [
        column * 5.0 + u(GUTTER * 0.5),
        top,
        column * 11.0,
        height - u(GUTTER),
    ];
    let tab_width = (right[2] - right[0]) * 0.5;
    paint.fill(
        [right[0], right[1], right[0] + tab_width, right[1] + u(20.0)],
        TAB_SELECTED,
    )?;
    paint.fill(
        [right[0] + tab_width, right[1], right[2], right[1] + u(20.0)],
        ROW,
    )?;
    paint.text(
        "Overview",
        [right[0] + u(6.0), right[1] + u(6.0)],
        tab_width,
        TEXT,
    )?;
    paint.text(
        "Stats",
        [right[0] + tab_width + u(6.0), right[1] + u(6.0)],
        tab_width,
        MUTED,
    )?;
    let rows = [
        ("Friends", profile.friends.to_string()),
        ("Followers", profile.followers.to_string()),
        ("Gamerscore", profile.gamerscore.to_string()),
    ];
    let mut row_top = right[1] + u(20.0 + GUTTER);
    for (label, value) in rows {
        paint.fill([right[0], row_top, right[2], row_top + u(ROW_HEIGHT)], ROW)?;
        let inner = right[2] - right[0] - u(12.0);
        paint.text(
            label,
            [right[0] + u(6.0), row_top + u(7.0)],
            inner * 0.6,
            TEXT,
        )?;
        paint.text(
            &value,
            [right[0] + u(6.0) + inner * 0.6, row_top + u(7.0)],
            inner * 0.4,
            MUTED,
        )?;
        row_top += u(ROW_HEIGHT + 4.0);
    }
    Ok(hits)
}
