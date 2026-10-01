//! OreUI components drawn from the theme: the screen overlay and header bar,
//! solid buttons (elevated, dropping 0.4rem when pressed), panels, dividers,
//! list rows, solid tabs, text fields, segmented controls and the switch.

use super::super::super::UiPresentationError;
use super::icons::{self, Icon};
use super::paint::{Bounds, Canvas};
use super::theme::{
    BEVEL_DARK, BEVEL_LIGHT, BODY, BORDER, CAPTION, DESTRUCTIVE, EDGE, FIELD_CARET,
    FIELD_PLACEHOLDER, HEADER_HEIGHT, HEADER_STRIP, HEADER5, NEUTRAL, NEUTRAL20, NEUTRAL80,
    NEUTRAL100, OUTLINE, OVERLAY_SCREEN, PRIMARY_BUTTON, PRIMARY_ROLE, Rgba, Role, SECONDARY,
    SECONDARY_BUTTON, TEXT, TEXT_DIMMER, Type,
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
    Destructive,
}

impl Variant {
    fn role(self) -> Role {
        match self {
            Self::Hero | Self::Primary => PRIMARY_ROLE,
            Self::Secondary => SECONDARY,
            Self::Neutral => NEUTRAL,
            Self::Destructive => DESTRUCTIVE,
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
    button_face(
        canvas,
        b,
        variant,
        label,
        Interaction::of(view, action),
        action.is_some(),
    )?;
    if let Some(action) = action {
        canvas.hit(action, b)?;
    }
    Ok(())
}

/// A solid button's art in `state`; the caller owns its hit area.
pub(super) fn button_face(
    canvas: &mut Canvas<'_>,
    b: Bounds,
    variant: Variant,
    label: &str,
    state: Interaction,
    enabled: bool,
) -> Result<(), UiPresentationError> {
    let role = variant.role();
    let disabled = !enabled;
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
    canvas.text_centred(label, face, variant.label(), text, shadowed)
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

/// Bevelled face colours: border, top-left edge, bottom-right edge, fill.
struct Bevel {
    edge_light: Rgba,
    edge_dark: Rgba,
    fill: Rgba,
}

const ROW_IDLE: Bevel = Bevel {
    edge_light: rgb(0x5a5b5c),
    edge_dark: rgb(0x323334),
    fill: rgb(0x48494a),
};
const ROW_HOVER: Bevel = Bevel {
    edge_light: rgb(0x69696b),
    edge_dark: rgb(0x3e3e3f),
    fill: rgb(0x58585a),
};
const ROW_PRESSED: Bevel = Bevel {
    edge_light: rgb(0x464747),
    edge_dark: rgb(0x222324),
    fill: rgb(0x313233),
};
const TAB_IDLE: Bevel = Bevel {
    edge_light: rgb(0x6d6d6e),
    edge_dark: rgb(0x5a5b5c),
    fill: rgb(0x48494a),
};
const TAB_HOVER: Bevel = Bevel {
    edge_light: rgb(0x79797b),
    edge_dark: rgb(0x69696b),
    fill: rgb(0x58585a),
};
const TAB_SELECTED: Bevel = Bevel {
    edge_light: rgb(0x5a5b5c),
    edge_dark: rgb(0x464747),
    fill: rgb(0x313233),
};

const fn rgb(value: u32) -> Rgba {
    [(value >> 16) as u8, (value >> 8) as u8, value as u8, 255]
}

/// A bevelled face inside a dark border; `front` adds the lower front face strip.
fn bevelled(
    canvas: &mut Canvas<'_>,
    b: Bounds,
    bevel: &Bevel,
    front: bool,
) -> Result<(), UiPresentationError> {
    canvas.fill(b, BORDER)?;
    let edge = canvas.r(EDGE);
    let inner = [b[0] + edge, b[1] + edge, b[2] - edge, b[3] - edge];
    let face = if front {
        let strip = canvas.r(0.4);
        canvas.fill(
            [inner[0], inner[3] - strip, inner[2], inner[3]],
            NEUTRAL80.fill,
        )?;
        [inner[0], inner[1], inner[2], inner[3] - strip]
    } else {
        inner
    };
    canvas.fill(face, bevel.fill)?;
    canvas.specular(face, bevel.edge_light, bevel.edge_dark)
}

/// A world or server list row: a bevelled action face that lightens on hover.
pub(super) fn row(
    canvas: &mut Canvas<'_>,
    view: &MenuView,
    b: Bounds,
    selected: bool,
    action: Option<MenuAction>,
) -> Result<(), UiPresentationError> {
    let state = Interaction::of(view, action);
    let bevel = if state.pressed {
        &ROW_PRESSED
    } else if state.hovered || selected {
        &ROW_HOVER
    } else {
        &ROW_IDLE
    };
    bevelled(canvas, b, bevel, false)?;
    if state.focused {
        let ring = canvas.r(0.4);
        canvas.frame(
            [b[0] - ring, b[1] - ring, b[2] + ring, b[3] + ring],
            EDGE,
            OUTLINE,
        )?;
    }
    if let Some(action) = action {
        canvas.hit(action, b)?;
    }
    Ok(())
}

/// The bevelled tab bar: raised tabs with a front face; the selected one sits
/// 0.4rem lower, darker, with a white indicator under its centre.
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
    let overlap = canvas.r(EDGE);
    let width = (b[2] - b[0] + overlap * (labels.len() - 1) as f32) / labels.len() as f32;
    for (index, (label, action)) in labels.iter().enumerate() {
        let left = b[0] + (width - overlap) * index as f32;
        let cell = [left, b[1], left + width, b[3]];
        if index == selected {
            let face = [cell[0], cell[1] + canvas.r(0.4), cell[2], cell[3]];
            bevelled(canvas, face, &TAB_SELECTED, false)?;
            let indicator = canvas.r(4.8).min(face[2] - face[0]);
            let centre = (face[0] + face[2]) * 0.5;
            canvas.fill(
                [
                    centre - indicator * 0.5,
                    face[3],
                    centre + indicator * 0.5,
                    face[3] + canvas.r(EDGE),
                ],
                OUTLINE,
            )?;
            canvas.text_centred(label, face, BODY, NEUTRAL.text, false)?;
            continue;
        }
        let state = Interaction::of(view, *action);
        bevelled(
            canvas,
            cell,
            if state.hovered { &TAB_HOVER } else { &TAB_IDLE },
            true,
        )?;
        if state.focused {
            canvas.frame(
                [
                    cell[0] - overlap,
                    cell[1] - overlap,
                    cell[2] + overlap,
                    cell[3] + overlap,
                ],
                EDGE,
                OUTLINE,
            )?;
        }
        let face = [cell[0], cell[1], cell[2], cell[3] - canvas.r(0.4)];
        canvas.text_centred(label, face, BODY, NEUTRAL.text, false)?;
        if let Some(action) = action {
            canvas.hit(*action, cell)?;
        }
    }
    Ok(())
}

/// The translucent side menu panel with its border.
pub(super) fn side_menu(canvas: &mut Canvas<'_>, b: Bounds) -> Result<(), UiPresentationError> {
    canvas.fill(b, [0, 0, 0, 153])?;
    canvas.frame(b, EDGE, BORDER)
}

/// A side-menu section label (bottom-aligned caption over a divider); returns its bottom.
pub(super) fn section_label(
    canvas: &mut Canvas<'_>,
    label: &str,
    span: [f32; 2],
    top: f32,
) -> Result<f32, UiPresentationError> {
    let height = canvas.r(4.8);
    let pad = canvas.r(1.6);
    let text_top = top + height - canvas.r(0.8) - canvas.r(CAPTION.line);
    let width = span[1] - span[0] - pad * 2.0;
    // The open font runs wider than vanilla's; shrink a long label to fit whole.
    let natural = canvas.measure(label, CAPTION)?;
    let style = if natural > width {
        Type {
            size: CAPTION.size * (width / natural * 0.98).max(0.6),
            ..CAPTION
        }
    } else {
        CAPTION
    };
    canvas.text_line(label, [span[0] + pad, text_top], width, style, TEXT_DIMMER)?;
    divider(canvas, span[0], span[1], top + height - canvas.r(EDGE))?;
    Ok(top + height)
}

/// A list row's title and caption, one line each, centred in a 4.8rem row.
pub(super) fn row_text(
    canvas: &mut Canvas<'_>,
    [left, top]: [f32; 2],
    width: f32,
    title: &str,
    caption: &str,
) -> Result<(), UiPresentationError> {
    let pad = (4.8 - BODY.line - CAPTION.line) * 0.5;
    let glyph = |style: Type| (style.line - style.size) * 0.5;
    canvas.text_line(
        title,
        [left, top + canvas.r(pad + glyph(BODY))],
        width,
        BODY,
        TEXT,
    )?;
    canvas.text_line(
        caption,
        [left, top + canvas.r(pad + BODY.line + glyph(CAPTION))],
        width,
        CAPTION,
        TEXT_DIMMER,
    )?;
    Ok(())
}

/// A small solid tag; returns its right edge.
pub(super) fn tag(
    canvas: &mut Canvas<'_>,
    label: &str,
    at: [f32; 2],
    fill: Rgba,
    text: Rgba,
) -> Result<f32, UiPresentationError> {
    let pad = canvas.r(0.4);
    let width = canvas.measure(label, BODY)? + pad * 2.0;
    let b = [at[0], at[1], at[0] + width, at[1] + canvas.r(2.0)];
    canvas.fill(b, fill)?;
    canvas.text(label, [b[0] + pad, b[1]], width, BODY, text, false)?;
    Ok(b[2])
}

/// A text field: a dark face inside the field border (0.6rem on top), the value or placeholder
/// at the field's padding, and the caret while focused. The border art is approximated.
pub(super) fn text_field(
    canvas: &mut Canvas<'_>,
    view: &MenuView,
    b: Bounds,
    value: &str,
    placeholder: &str,
    focused: bool,
    action: MenuAction,
) -> Result<(), UiPresentationError> {
    let state = Interaction::of(view, Some(action));
    canvas.fill(b, BORDER)?;
    let edge = canvas.r(EDGE);
    let face = [b[0] + edge, b[1] + canvas.r(0.6), b[2] - edge, b[3] - edge];
    canvas.fill(
        face,
        if state.hovered {
            NEUTRAL80.hovered
        } else {
            NEUTRAL100
        },
    )?;
    let left = b[0] + canvas.r(1.4);
    let width = (b[2] - canvas.r(1.2) - left).max(1.0);
    let top = face[1] + (face[3] - face[1] - canvas.r(BODY.line)) * 0.5;
    let shown = if value.is_empty() { placeholder } else { value };
    let color = if value.is_empty() {
        FIELD_PLACEHOLDER
    } else {
        TEXT
    };
    canvas.text_line(shown, [left, top], width, BODY, color)?;
    if focused {
        let caret_x = if value.is_empty() {
            left
        } else {
            left + canvas.measure(value, BODY)?.min(width)
        };
        canvas.fill(
            [caret_x, top, caret_x + edge, top + canvas.r(BODY.line)],
            FIELD_CARET,
        )?;
        canvas.frame(b, EDGE, OUTLINE)?;
    } else if state.focused {
        let ring = canvas.r(0.4);
        canvas.frame(
            [b[0] - ring, b[1] - ring, b[2] + ring, b[3] + ring],
            EDGE,
            OUTLINE,
        )?;
    }
    canvas.hit(action, b)
}

/// A segmented control: one bevelled cell per option, the selected one sunk and dark.
pub(super) fn segmented(
    canvas: &mut Canvas<'_>,
    view: &MenuView,
    b: Bounds,
    options: &[(&str, MenuAction, bool)],
) -> Result<(), UiPresentationError> {
    let labels: Vec<(&str, Option<MenuAction>)> = options
        .iter()
        .map(|(label, action, _)| (*label, Some(*action)))
        .collect();
    let selected = options
        .iter()
        .position(|(_, _, on)| *on)
        .unwrap_or(usize::MAX);
    tabs(canvas, view, b, &labels, selected)
}

/// A switch: a track with the knob right (green) when on, left (grey) when off.
pub(super) fn switch(
    canvas: &mut Canvas<'_>,
    view: &MenuView,
    b: Bounds,
    on: bool,
    action: MenuAction,
) -> Result<(), UiPresentationError> {
    let state = Interaction::of(view, Some(action));
    canvas.fill(b, BORDER)?;
    let edge = canvas.r(EDGE);
    let track = [b[0] + edge, b[1] + edge, b[2] - edge, b[3] - edge];
    canvas.fill(
        track,
        if on {
            PRIMARY_ROLE.fill
        } else {
            NEUTRAL80.fill
        },
    )?;
    let knob_width = (track[3] - track[1]).min((track[2] - track[0]) * 0.5);
    let knob_left = if on { track[2] - knob_width } else { track[0] };
    let knob = [knob_left, track[1], knob_left + knob_width, track[3]];
    canvas.fill(knob, BORDER)?;
    let face = [
        knob[0] + edge,
        knob[1] + edge,
        knob[2] - edge,
        knob[3] - edge,
    ];
    canvas.fill(
        face,
        if state.hovered {
            SECONDARY.fill
        } else {
            SECONDARY.hovered
        },
    )?;
    canvas.specular(face, SECONDARY.specular_top, SECONDARY.specular_bottom)?;
    if state.focused {
        let ring = canvas.r(0.4);
        canvas.frame(
            [b[0] - ring, b[1] - ring, b[2] + ring, b[3] + ring],
            EDGE,
            OUTLINE,
        )?;
    }
    canvas.hit(action, b)
}
