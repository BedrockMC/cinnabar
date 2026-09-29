//! OreUI components drawn from the theme: the screen overlay and header bar,
//! solid buttons (elevated, dropping 0.4rem when pressed), panels, dividers,
//! list rows, solid tabs, the switch and the slider.

use super::super::super::UiPresentationError;
use super::icons::{self, Icon};
use super::paint::{Bounds, Canvas};
use super::theme::{
    BEVEL_DARK, BEVEL_LIGHT, BODY, BORDER, EDGE, HEADER_HEIGHT, HEADER_STRIP, HEADER5, NEUTRAL,
    NEUTRAL20, NEUTRAL80, OUTLINE, OVERLAY_SCREEN, PRIMARY, PRIMARY_BUTTON, PRIMARY_ROLE, Role,
    SECONDARY, SECONDARY_BUTTON, Type,
};
use crate::menu::{MenuAction, MenuView};

/// How a control is being interacted with this frame.
#[derive(Clone, Copy, Default)]
pub(super) struct Interaction {
    pub(super) hovered: bool,
    pub(super) pressed: bool,
    pub(super) focused: bool,
}

impl Interaction {
    pub(super) fn of(view: &MenuView, action: Option<MenuAction>) -> Self {
        let Some(action) = action else {
            return Self::default();
        };
        Self {
            hovered: view.hovered == Some(action),
            pressed: view.pressed == Some(action),
            focused: view.focused_action == Some(action),
        }
    }
}

/// A solid button's colour variant.
#[derive(Clone, Copy)]
pub(super) enum Variant {
    /// Primary colours with the large heading label.
    Hero,
    Primary,
    Secondary,
    Neutral,
}

impl Variant {
    fn role(self) -> Role {
        match self {
            Self::Hero | Self::Primary => PRIMARY_ROLE,
            Self::Secondary => SECONDARY,
            Self::Neutral => NEUTRAL,
        }
    }

    fn label(self) -> Type {
        match self {
            Self::Hero => PRIMARY_BUTTON,
            _ => SECONDARY_BUTTON,
        }
    }
}

/// The dimming overlay every OreUI screen draws over the world or panorama.
pub(super) fn screen_overlay(
    canvas: &mut Canvas<'_>,
    size: [f32; 2],
) -> Result<(), UiPresentationError> {
    canvas.fill([0.0, 0.0, size[0], size[1]], OVERLAY_SCREEN)
}

/// The light header bar with an optional back button; returns its bottom edge.
pub(super) fn header(
    canvas: &mut Canvas<'_>,
    view: &MenuView,
    title: &str,
    width: f32,
    back: Option<MenuAction>,
) -> Result<f32, UiPresentationError> {
    let row = canvas.r(4.4);
    let strip = canvas.r(0.4);
    canvas.fill([0.0, 0.0, width, row], NEUTRAL20.fill)?;
    canvas.bevel(
        [0.0, 0.0, width, row],
        NEUTRAL20.specular_top,
        NEUTRAL20.specular_bottom,
    )?;
    canvas.fill([0.0, row, width, row + strip], HEADER_STRIP)?;
    canvas.fill(
        [0.0, row + strip, width, row + strip + canvas.r(EDGE)],
        BEVEL_DARK,
    )?;
    let pad = canvas.r(6.0);
    canvas.text_centred(
        title,
        [pad, 0.0, width - pad, row],
        HEADER5,
        NEUTRAL20.text,
        false,
    )?;
    if let Some(action) = back {
        let inset = canvas.r(EDGE);
        let button = [inset, inset, inset + canvas.r(4.0), row - inset];
        let state = Interaction::of(view, Some(action));
        let fill = if state.pressed {
            NEUTRAL20.pressed
        } else if state.hovered {
            NEUTRAL20.hovered
        } else {
            NEUTRAL20.fill
        };
        canvas.fill(button, fill)?;
        if state.focused {
            canvas.frame(button, EDGE, [0, 0, 0, 255])?;
        }
        let [texel_w, texel_h] = Icon::ArrowBack.texels();
        let texel = canvas.r(EDGE);
        let at = [
            (button[0] + button[2] - texel_w as f32 * texel) * 0.5,
            (button[1] + button[3] - texel_h as f32 * texel) * 0.5,
        ];
        icons::draw(canvas, Icon::ArrowBack, at, NEUTRAL20.text)?;
        canvas.hit(action, button)?;
    }
    Ok(canvas.r(HEADER_HEIGHT) + canvas.r(EDGE))
}

/// A solid, elevated button: the face drops 0.4rem into its shadow strip when pressed.
pub(super) fn button(
    canvas: &mut Canvas<'_>,
    view: &MenuView,
    b: Bounds,
    variant: Variant,
    label: &str,
    action: Option<MenuAction>,
) -> Result<(), UiPresentationError> {
    let role = variant.role();
    let state = Interaction::of(view, action);
    let disabled = action.is_none();
    let drop = if state.pressed { canvas.r(0.4) } else { 0.0 };
    let shadow = canvas.r(0.4);
    let face = [b[0], b[1] + drop, b[2], b[3] - shadow + drop];
    if !state.pressed {
        canvas.fill([b[0], b[3] - shadow, b[2], b[3]], role.shadow)?;
    }
    let fill = if disabled {
        [0xb1, 0xb2, 0xb5, 255]
    } else if state.pressed {
        role.pressed
    } else if state.hovered {
        role.hovered
    } else {
        role.fill
    };
    canvas.fill(face, fill)?;
    let (top, bottom) = if state.hovered {
        (role.specular_top_hovered, role.specular_bottom_hovered)
    } else {
        (role.specular_top, role.specular_bottom)
    };
    let inner = canvas.r(EDGE);
    canvas.specular(
        [
            face[0] + inner,
            face[1] + inner,
            face[2] - inner,
            face[3] - inner,
        ],
        top,
        bottom,
    )?;
    canvas.frame([face[0], face[1], face[2], face[3]], EDGE, BORDER)?;
    if state.focused {
        let ring = canvas.r(0.4);
        canvas.frame(
            [b[0] - ring, b[1] - ring, b[2] + ring, b[3] + ring],
            EDGE,
            OUTLINE,
        )?;
    }
    let text = if disabled {
        [0x58, 0x58, 0x5a, 255]
    } else {
        role.text
    };
    let shadowed = matches!(variant, Variant::Hero);
    canvas.text_centred(label, face, variant.label(), text, shadowed)?;
    if let Some(action) = action {
        canvas.hit(action, b)?;
    }
    Ok(())
}

/// A neutral80 panel with the dark one-texel border.
pub(super) fn panel(canvas: &mut Canvas<'_>, b: Bounds) -> Result<(), UiPresentationError> {
    canvas.fill(b, NEUTRAL80.fill)?;
    canvas.frame(b, EDGE, BORDER)
}

/// A one-texel divider with reversed bevel edges.
pub(super) fn divider(
    canvas: &mut Canvas<'_>,
    left: f32,
    right: f32,
    y: f32,
) -> Result<(), UiPresentationError> {
    let w = canvas.r(EDGE) * 0.5;
    canvas.fill([left, y, right, y + w], BEVEL_DARK)?;
    canvas.fill([left, y + w, right, y + w * 2.0], BEVEL_LIGHT)
}

/// A neutral list row: hover and selection lighten it; returns nothing but draws the bevel.
pub(super) fn row(
    canvas: &mut Canvas<'_>,
    view: &MenuView,
    b: Bounds,
    selected: bool,
    action: Option<MenuAction>,
) -> Result<(), UiPresentationError> {
    let state = Interaction::of(view, action);
    let fill = if state.pressed {
        NEUTRAL.pressed
    } else if state.hovered || selected {
        NEUTRAL.hovered
    } else {
        NEUTRAL.fill
    };
    canvas.fill(b, fill)?;
    canvas.bevel(b, BEVEL_LIGHT, BEVEL_DARK)?;
    if state.focused {
        canvas.frame(b, EDGE, OUTLINE)?;
    }
    if let Some(action) = action {
        canvas.hit(action, b)?;
    }
    Ok(())
}

/// Solid tabs across `b`; the selected one is green with a white underline.
pub(super) fn tabs(
    canvas: &mut Canvas<'_>,
    view: &MenuView,
    b: Bounds,
    labels: &[(&str, Option<MenuAction>)],
    selected: usize,
) -> Result<(), UiPresentationError> {
    if labels.is_empty() {
        return Ok(());
    }
    let width = (b[2] - b[0]) / labels.len() as f32;
    for (index, (label, action)) in labels.iter().enumerate() {
        let cell = [
            b[0] + width * index as f32,
            b[1],
            b[0] + width * (index + 1) as f32,
            b[3],
        ];
        if index == selected {
            let lift = canvas.r(0.4);
            let face = [cell[0], cell[1] + lift, cell[2], cell[3]];
            canvas.fill(face, PRIMARY)?;
            canvas.frame(face, EDGE, BORDER)?;
            let underline = canvas.r(EDGE);
            let inset = canvas.r(1.2);
            canvas.fill(
                [
                    face[0] + inset,
                    face[3] - underline * 3.0,
                    face[2] - inset,
                    face[3] - underline * 2.0,
                ],
                OUTLINE,
            )?;
            canvas.text_centred(label, face, BODY, PRIMARY_ROLE.text, false)?;
        } else {
            button(canvas, view, cell, Variant::Secondary, label, *action)?;
        }
    }
    Ok(())
}
