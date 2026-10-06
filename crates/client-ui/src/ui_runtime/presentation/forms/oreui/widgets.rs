//! OreUI components drawn from the theme: the screen overlay and header bar,
//! solid buttons (elevated, dropping 0.4rem when pressed), panels, dividers,
//! list rows, modal menu items, solid tabs, text fields and segmented controls.

use super::super::super::{IconRef, UiPresentationError};
use super::super::menu_caret::{TextSpot, caret_byte};
use super::icons::{self, Icon};
use super::paint::{Bounds, Canvas, text_factor};
use super::theme::{
    BEVEL_DARK, BEVEL_LIGHT, BODY, BORDER, Bundle, CAPTION, DESTRUCTIVE, DISABLED, EDGE,
    FIELD_CARET, FIELD_PLACEHOLDER, HEADER_HEIGHT, HEADER_STRIP, HEADER5, MENU_DESTRUCTIVE,
    MENU_ITEM, MENU_ITEM_DISABLED, MENU_NEUTRAL, MENU_SECONDARY, MENU_SPECULAR_ACTIVE, NEUTRAL,
    NEUTRAL20, NEUTRAL80, NEUTRAL100, OUTLINE, OVERLAY_SCREEN, PRIMARY_BUTTON, PRIMARY_ROLE, Rgba,
    Role, SECONDARY, SECONDARY_BUTTON, TEXT, TEXT_DIMMER, TEXT_DIMMEST, Type,
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
    fn role(self, bundle: Bundle) -> Role {
        match (bundle, self) {
            (_, Self::Hero | Self::Primary) => PRIMARY_ROLE,
            (Bundle::Menus, Self::Secondary) => MENU_SECONDARY,
            (Bundle::Menus, Self::Neutral) => MENU_NEUTRAL,
            (Bundle::Menus, Self::Destructive) => MENU_DESTRUCTIVE,
            (Bundle::Gameplay, Self::Secondary) => SECONDARY,
            (Bundle::Gameplay, Self::Neutral) => NEUTRAL,
            (Bundle::Gameplay, Self::Destructive) => DESTRUCTIVE,
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
        NEUTRAL20.specular[0],
        NEUTRAL20.specular[1],
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

/// A solid button's art in `state`; the caller owns its hit area. The one-texel outline
/// wraps the face and its 0.4rem shadow strip; a focused button adds a ring just outside it.
pub(super) fn button_face(
    canvas: &mut Canvas<'_>,
    b: Bounds,
    variant: Variant,
    label: &str,
    state: Interaction,
    enabled: bool,
) -> Result<(), UiPresentationError> {
    let role = if enabled {
        variant.role(canvas.bundle)
    } else {
        DISABLED
    };
    let pressed = enabled && state.pressed;
    let hovered = enabled && state.hovered && !pressed;
    let focused = enabled && state.focused;
    // Menus art draws pressed and (unhovered) focused faces from their own nine-slices.
    let active = canvas.bundle == Bundle::Menus && (pressed || (focused && !hovered));
    let edge = canvas.r(EDGE);
    let shadow = if pressed { 0.0 } else { canvas.r(0.4) };
    let outer = if pressed {
        [b[0], b[1] + canvas.r(0.4), b[2], b[3]]
    } else {
        b
    };
    canvas.fill(outer, if active { BORDER } else { role.border })?;
    let inner = [
        outer[0] + edge,
        outer[1] + edge,
        outer[2] - edge,
        outer[3] - edge,
    ];
    let face = [inner[0], inner[1], inner[2], inner[3] - shadow];
    canvas.fill([face[0], face[3], face[2], inner[3]], role.shadow)?;
    let fill = if pressed {
        role.pressed
    } else if hovered {
        role.hovered
    } else {
        role.fill
    };
    canvas.fill(face, fill)?;
    let [top, bottom] = if active {
        MENU_SPECULAR_ACTIVE
    } else if hovered {
        role.specular_hovered
    } else {
        role.specular
    };
    canvas.specular(face, top, bottom)?;
    if focused {
        canvas.frame(
            [
                outer[0] - edge,
                outer[1] - edge,
                outer[2] + edge,
                outer[3] + edge,
            ],
            EDGE,
            OUTLINE,
        )?;
    }
    let shadowed = matches!(variant, Variant::Hero) && enabled;
    canvas.text_centred(
        label,
        [outer[0], outer[1], outer[2], outer[3] - shadow],
        variant.label(),
        role.text,
        shadowed,
    )
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

/// A bevelled face: its fill and the translucent top-left and bottom-right edges over it.
struct Bevel {
    fill: Rgba,
    edges: [Rgba; 2],
}

/// List rows take the neutral bevel; tabs the neutral speculars.
const ROW_IDLE: Bevel = Bevel {
    fill: NEUTRAL.fill,
    edges: [BEVEL_LIGHT, BEVEL_DARK],
};
const ROW_HOVER: Bevel = Bevel {
    fill: NEUTRAL.hovered,
    ..ROW_IDLE
};
const ROW_PRESSED: Bevel = Bevel {
    fill: NEUTRAL.pressed,
    ..ROW_IDLE
};
const TAB_IDLE: Bevel = Bevel {
    fill: NEUTRAL.fill,
    edges: NEUTRAL.specular,
};
const TAB_HOVER: Bevel = Bevel {
    fill: NEUTRAL.hovered,
    ..TAB_IDLE
};
const TAB_SELECTED: Bevel = Bevel {
    fill: NEUTRAL.pressed,
    ..TAB_IDLE
};

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
    canvas.specular(face, bevel.edges[0], bevel.edges[1])
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

/// One modal menu item.
pub(super) struct MenuItem<'a> {
    pub(super) label: &'a str,
    /// Reserves the picture slot; a missing picture shows the player silhouette.
    pub(super) picture_slot: bool,
    pub(super) picture: Option<IconRef>,
    pub(super) selected: bool,
    pub(super) enabled: bool,
    pub(super) action: Option<MenuAction>,
}

/// A 4.8rem modal menu item, bordered above and at the sides (the list closes the bottom):
/// a 2.4rem picture, the label, and a check at the right while selected.
pub(super) fn menu_item(
    canvas: &mut Canvas<'_>,
    view: &MenuView,
    b: Bounds,
    item: &MenuItem<'_>,
) -> Result<(), UiPresentationError> {
    let action = item.action.filter(|_| item.enabled);
    let state = Interaction::of(view, action);
    let role = if item.enabled {
        MENU_ITEM
    } else {
        MENU_ITEM_DISABLED
    };
    let edge = canvas.r(EDGE);
    canvas.fill(b, role.border)?;
    let face = [b[0] + edge, b[1] + edge, b[2] - edge, b[3]];
    let fill = if state.pressed {
        role.pressed
    } else if state.hovered {
        role.hovered
    } else {
        role.fill
    };
    canvas.fill(face, fill)?;
    if state.focused {
        canvas.frame(face, EDGE, OUTLINE)?;
    }
    let pad = canvas.r(1.6);
    let middle = (face[1] + face[3]) * 0.5;
    let mut left = face[0] + pad;
    if item.picture_slot || item.picture.is_some() {
        let side = canvas.r(2.4);
        if let Some(picture) = item.picture {
            canvas.icon_ref(
                picture,
                [left, middle - side * 0.5, left + side, middle + side * 0.5],
            )?;
        } else {
            let [w, h] = Icon::Player.texels().map(|texels| texels as f32 * edge);
            let at = [left + (side - w) * 0.5, middle - h * 0.5];
            icons::draw(canvas, Icon::Player, at, TEXT_DIMMEST)?;
        }
        left += side + canvas.r(0.8);
    }
    let [check_w, check_h] = Icon::Check.texels().map(|texels| texels as f32 * edge);
    let check_left = face[2] - pad - check_w;
    let top = middle - canvas.r(BODY.line) * 0.5 + canvas.r((BODY.line - BODY.size) * 0.5);
    canvas.text_line(
        item.label,
        [left, top],
        check_left - canvas.r(0.8) - left,
        BODY,
        role.text,
    )?;
    if item.selected {
        icons::draw(
            canvas,
            Icon::Check,
            [check_left, middle - check_h * 0.5],
            role.text,
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
/// at the field's padding, and the caret at its position while focused. The border art is
/// approximated.
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
        let before = &value[..caret_byte(value, view.caret.byte)];
        let caret_x = if before.is_empty() {
            left
        } else {
            left + canvas.measure(before, BODY)?.min(width)
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
    canvas.hit(action, b)?;
    // A press inside the field places its caret by character.
    if let Some(field) = action.text_field()
        && let Some(&(hit, bounds)) = canvas.hits.last()
        && hit == action
    {
        let metrics = canvas.metrics;
        canvas.spots.push(TextSpot {
            field,
            bounds,
            left,
            factor: text_factor(BODY),
            font: None,
            metrics,
        });
    }
    Ok(())
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

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::super::review_tests::{paint, solids};
    use super::super::theme::{MENU_SECONDARY, MENU_SPECULAR_ACTIVE};
    use super::*;

    /// One 40×4.4rem secondary face's fills and the logical px per rem.
    fn face(bundle: Bundle, state: Interaction, enabled: bool) -> (Vec<(Bounds, Rgba)>, f32) {
        let mut rem = 0.0;
        let (_, _, nodes) = paint(HashMap::new(), |canvas| {
            canvas.bundle = bundle;
            rem = canvas.rem;
            let b = [0.0, 0.0, canvas.r(40.0), canvas.r(4.4)];
            button_face(canvas, b, Variant::Secondary, "", state, enabled).unwrap();
        });
        (solids(&nodes), rem)
    }

    fn has(fills: &[(Bounds, Rgba)], color: Rgba, b: Bounds) -> bool {
        fills
            .iter()
            .any(|(at, c)| *c == color && at.iter().zip(b).all(|(x, y)| (x - y).abs() < 0.01))
    }

    #[test]
    fn menus_secondary_outline_wraps_face_and_shadow_in_black() {
        let (fills, rem) = face(Bundle::Menus, Interaction::default(), true);
        let [w, h, e, s] = [40.0 * rem, 4.4 * rem, EDGE * rem, 0.4 * rem];
        assert!(
            has(&fills, MENU_SECONDARY.border, [0.0, 0.0, w, h]),
            "{fills:?}"
        );
        assert!(has(
            &fills,
            MENU_SECONDARY.shadow,
            [e, h - e - s, w - e, h - e]
        ));
        assert!(has(&fills, MENU_SECONDARY.fill, [e, e, w - e, h - e - s]));
        assert!(has(
            &fills,
            MENU_SECONDARY.specular[0],
            [e, e, w - e, e * 2.0]
        ));
    }

    #[test]
    fn pressed_menus_face_drops_into_its_shadow_with_the_active_art() {
        let pressed = Interaction {
            pressed: true,
            ..Interaction::default()
        };
        let (fills, rem) = face(Bundle::Menus, pressed, true);
        let [w, h, e, s] = [40.0 * rem, 4.4 * rem, EDGE * rem, 0.4 * rem];
        assert!(has(&fills, BORDER, [0.0, s, w, h]), "{fills:?}");
        assert!(has(
            &fills,
            MENU_SECONDARY.pressed,
            [e, s + e, w - e, h - e]
        ));
        assert!(has(
            &fills,
            MENU_SPECULAR_ACTIVE[0],
            [e, s + e, w - e, s + e * 2.0]
        ));
        assert!(!fills.iter().any(|(_, c)| *c == MENU_SECONDARY.shadow));
    }

    #[test]
    fn disabled_face_has_its_own_outline_and_shadow_without_speculars() {
        let (fills, rem) = face(Bundle::Menus, Interaction::default(), false);
        let [w, h] = [40.0 * rem, 4.4 * rem];
        assert!(has(&fills, DISABLED.border, [0.0, 0.0, w, h]));
        assert!(fills.iter().any(|(_, c)| *c == DISABLED.shadow));
        assert!(fills.iter().all(|(_, c)| c[3] == 255), "{fills:?}");
    }

    #[test]
    fn gameplay_secondary_keeps_the_role_table_art() {
        let (fills, rem) = face(Bundle::Gameplay, Interaction::default(), true);
        let [w, h, e] = [40.0 * rem, 4.4 * rem, EDGE * rem];
        assert!(has(&fills, BORDER, [0.0, 0.0, w, h]));
        assert!(has(&fills, SECONDARY.specular[0], [e, e, w - e, e * 2.0]));
    }

    #[test]
    fn focus_ring_hugs_the_outline() {
        let focused = Interaction {
            focused: true,
            ..Interaction::default()
        };
        let (fills, rem) = face(Bundle::Menus, focused, true);
        let [w, e] = [40.0 * rem, EDGE * rem];
        assert!(has(&fills, OUTLINE, [-e, -e, w + e, 0.0]), "{fills:?}");
    }

    #[test]
    fn specular_corners_blend_both_edges() {
        let (top, bottom) = ([255, 0, 0, 51], [0, 0, 255, 26]);
        let mut e = 0.0;
        let (_, _, nodes) = paint(HashMap::new(), |canvas| {
            e = canvas.r(EDGE);
            canvas
                .specular([0.0, 0.0, 100.0, 100.0], top, bottom)
                .unwrap();
        });
        let fills = solids(&nodes);
        let covers = |color: Rgba, [x, y]: [f32; 2]| {
            fills
                .iter()
                .any(|(b, c)| *c == color && b[0] <= x && x < b[2] && b[1] <= y && y < b[3])
        };
        let half = e * 0.5;
        for corner in [[100.0 - half, half], [half, 100.0 - half]] {
            assert!(covers(top, corner) && covers(bottom, corner), "{corner:?}");
        }
        assert!(covers(top, [half, half]) && !covers(bottom, [half, half]));
        let far = [100.0 - half, 100.0 - half];
        assert!(covers(bottom, far) && !covers(top, far));
    }
}
