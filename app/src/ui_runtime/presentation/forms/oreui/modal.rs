//! OreUI's modal (`Ug`): an overlay with a bordered neutral panel, a header with the title and
//! an X close button, caption text, and a vertical stack of full-width buttons. The local-world
//! dialogs (delete, unsaved changes, Docker, EULA, errors, the create spinner) are built on it.

use super::super::super::UiPresentationError;
use super::grid::space;
use super::icons::{self, Icon};
use super::paint::{Bounds, Canvas};
use super::theme::{BODY, CAPTION, EDGE, NEUTRAL80, OVERLAY_SCREEN, TEXT, TEXT_DIMMER};
use super::widgets::{Interaction, Variant, button, divider, panel};
use crate::local_worlds::{PromptButton, Screen, WorldsView};
use crate::menu::{LocalWorldAction, MenuAction, MenuView};

/// One modal: title, body and buttons, with the header X bound to `close`.
pub(super) struct Modal<'a> {
    pub(super) title: &'a str,
    pub(super) body: &'a str,
    pub(super) buttons: Vec<(&'a str, Variant, MenuAction)>,
    pub(super) close: Option<MenuAction>,
}

const WIDTH: f32 = 44.0;
const BUTTON: f32 = 4.4;

/// Draws `modal` centred over the screen; only its own controls take presses.
pub(super) fn draw(
    canvas: &mut Canvas<'_>,
    view: &MenuView,
    size: [f32; 2],
    modal: &Modal<'_>,
) -> Result<(), UiPresentationError> {
    canvas.hits.clear();
    canvas.fill([0.0, 0.0, size[0], size[1]], OVERLAY_SCREEN)?;
    let width = canvas.r(WIDTH).min(size[0] - space(canvas, 4));
    let pad = space(canvas, 3);
    let inner = width - pad * 2.0;
    let header = canvas.r(4.8);
    let body_height = if modal.body.is_empty() {
        0.0
    } else {
        canvas.measure_height(modal.body, inner, CAPTION)? + space(canvas, 3)
    };
    let buttons = modal.buttons.len() as f32;
    let buttons_height = buttons * canvas.r(BUTTON) + (buttons - 1.0).max(0.0) * space(canvas, 1);
    let height = header + pad + body_height + buttons_height + pad;
    let left = (size[0] - width) * 0.5;
    let top = ((size[1] - height) * 0.5).max(space(canvas, 2));
    panel(canvas, [left, top, left + width, top + height])?;
    canvas.text_line(
        modal.title,
        [left + pad, top + (header - canvas.r(BODY.line)) * 0.5],
        inner - canvas.r(4.0),
        BODY,
        TEXT,
    )?;
    if let Some(close) = modal.close {
        close_button(
            canvas,
            view,
            [left + width - header, top, left + width, top + header],
            close,
        )?;
    }
    divider(canvas, left, left + width, top + header - canvas.r(EDGE))?;
    let mut y = top + header + pad;
    if !modal.body.is_empty() {
        canvas.text(
            modal.body,
            [left + pad, y],
            inner,
            CAPTION,
            TEXT_DIMMER,
            false,
        )?;
        y += body_height;
    }
    for (label, variant, action) in &modal.buttons {
        let b = [left + pad, y, left + pad + inner, y + canvas.r(BUTTON)];
        button(canvas, view, b, *variant, label, Some(*action))?;
        y = b[3] + space(canvas, 1);
    }
    Ok(())
}

fn close_button(
    canvas: &mut Canvas<'_>,
    view: &MenuView,
    b: Bounds,
    action: MenuAction,
) -> Result<(), UiPresentationError> {
    let state = Interaction::of(view, Some(action));
    if state.hovered || state.pressed {
        canvas.fill(b, NEUTRAL80.hovered)?;
    }
    let [w, h] = Icon::Cross.texels();
    let texel = canvas.r(EDGE);
    let at = [
        (b[0] + b[2] - w as f32 * texel) * 0.5,
        (b[1] + b[3] - h as f32 * texel) * 0.5,
    ];
    icons::draw(canvas, Icon::Cross, at, TEXT)?;
    canvas.hit(action, b)
}

fn local(action: LocalWorldAction) -> MenuAction {
    MenuAction::LocalWorld(action)
}

/// The modal a local-world state shows, if any.
pub(super) fn local_world_modal(view: &WorldsView) -> Option<Modal<'_>> {
    let back = Some(local(LocalWorldAction::Back));
    Some(match view.screen {
        Screen::ConfirmDelete => Modal {
            title: "Are you sure?",
            body: "If you delete this world it will be gone forever.",
            buttons: vec![
                (
                    "Continue editing",
                    Variant::Secondary,
                    local(LocalWorldAction::Back),
                ),
                (
                    "Delete world",
                    Variant::Destructive,
                    local(LocalWorldAction::ConfirmDelete),
                ),
            ],
            close: back,
        },
        Screen::ConfirmLeaveEdit => Modal {
            title: "Do you want to save your changes?",
            body: "You have unsaved changes. Make sure to save or discard your changes.",
            buttons: vec![
                (
                    "Save changes",
                    Variant::Primary,
                    local(LocalWorldAction::Save),
                ),
                (
                    "Discard changes",
                    Variant::Secondary,
                    local(LocalWorldAction::Discard),
                ),
            ],
            close: back,
        },
        Screen::BackendPrompt => {
            let prompt = view.prompt?;
            Modal {
                title: prompt.title(),
                body: prompt.text(),
                buttons: prompt
                    .buttons()
                    .iter()
                    .map(|button| {
                        let variant = match button {
                            PromptButton::CreateFlat | PromptButton::Retry => Variant::Primary,
                            _ => Variant::Secondary,
                        };
                        (
                            button.label(),
                            variant,
                            local(LocalWorldAction::Prompt(*button)),
                        )
                    })
                    .collect(),
                close: back,
            }
        }
        Screen::Eula => Modal {
            title: "Minecraft End User License Agreement",
            body: "Default worlds run on Mojang's official Bedrock Dedicated Server, downloaded \
                   from minecraft.net the first time you play. Accept the Minecraft EULA and \
                   Privacy Policy to continue.",
            buttons: vec![
                (
                    "Accept",
                    Variant::Primary,
                    local(LocalWorldAction::AcceptEula),
                ),
                (
                    "View EULA",
                    Variant::Secondary,
                    local(LocalWorldAction::ViewEula),
                ),
                ("Cancel", Variant::Secondary, local(LocalWorldAction::Back)),
            ],
            close: back,
        },
        Screen::Error => Modal {
            title: "Something went wrong",
            body: view.error.as_deref().unwrap_or_default(),
            buttons: vec![("OK", Variant::Primary, local(LocalWorldAction::Back))],
            close: back,
        },
        Screen::Create if view.busy => Modal {
            title: "Creating new world...",
            body: "",
            buttons: Vec::new(),
            close: None,
        },
        _ => return None,
    })
}
