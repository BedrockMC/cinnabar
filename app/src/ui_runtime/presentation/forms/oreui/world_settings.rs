//! The local-world routes 1.26.50 draws with OreUI: Create New World (`/create-new-world`),
//! Edit world (`/edit-world`) and Create From Template (`/start-from-template`). Each is the
//! header with back, a side menu in four of twelve columns (preview, hero button, tab list)
//! and the tab's controls in eight. Only the tabs whose settings the core applies are shown.

use protocol::world_control::{Difficulty, GameMode, Generator};

use super::super::super::UiPresentationError;
use super::grid::{Grid, space};
use super::paint::{Bounds, Canvas};
use super::theme::{
    BODY, CAPTION, DESTRUCTIVE, EDGE, NEUTRAL100, SECONDARY_BUTTON, TEXT, TEXT_DIMMER, TEXT_DIMMEST,
};
use super::widgets::{
    Variant, button, header, panel, row, screen_overlay, segmented, switch, text_field,
};
use crate::local_worlds::{
    Screen, Tab, WorldsView, difficulty_description, difficulty_label, game_mode_description,
    game_mode_label,
};
use crate::menu::{LocalWorldAction as A, MenuAction, MenuField, MenuView};

const FIELD: f32 = 4.8;
const CONTROL: f32 = 4.4;

fn local(action: A) -> MenuAction {
    MenuAction::LocalWorld(action)
}

/// Which route a local-world state draws under its modal, if it covers the worlds tab.
pub(super) fn route(screen: Screen, view: &WorldsView) -> Option<Screen> {
    match screen {
        Screen::Create | Screen::Edit | Screen::Templates => Some(screen),
        Screen::ConfirmDelete | Screen::ConfirmLeaveEdit => Some(Screen::Edit),
        Screen::BackendPrompt
            if view
                .prompt
                .is_some_and(|p| p.blocking == crate::local_worlds::PromptFor::CreateDefault) =>
        {
            Some(Screen::Create)
        }
        _ => None,
    }
}

pub(super) fn draw(
    canvas: &mut Canvas<'_>,
    view: &MenuView,
    size: [f32; 2],
    route: Screen,
) -> Result<(), UiPresentationError> {
    let [width, height] = size;
    screen_overlay(canvas, size)?;
    let title = match route {
        Screen::Edit => "Edit world",
        Screen::Templates => "Create From Template",
        _ => "Create New World",
    };
    let top = header(canvas, view, title, width, Some(local(A::Back)))? + space(canvas, 2);
    let grid = Grid::new(canvas.r(1.0), width);
    let (side, content) = if grid.narrow {
        ((0, 3), (3, 5))
    } else {
        ((0, 4), (4, 8))
    };
    let side = grid.span(side.0, side.1);
    let content = grid.span(content.0, content.1);
    let bottom = height - space(canvas, 2);
    let local_view = &view.local;
    match route {
        Screen::Templates => {
            let mut y = top;
            button(
                canvas,
                view,
                [side[0], y, side[1], y + canvas.r(CONTROL)],
                Variant::Primary,
                "Create new world",
                Some(local(A::BeginCreate)),
            )?;
            y += canvas.r(CONTROL) + space(canvas, 2);
            tab_row(canvas, view, side, y, "Owned by me (0)", true, None)?;
            templates_empty(canvas, view, [content[0], top, content[1], bottom])
        }
        Screen::Edit => {
            let y = side_preview(canvas, side, top)?;
            button(
                canvas,
                view,
                [side[0], y, side[1], y + canvas.r(5.6)],
                Variant::Hero,
                "Play",
                Some(local(A::PlayFromEdit)),
            )?;
            let y = y + canvas.r(5.6) + space(canvas, 2);
            tab_row(canvas, view, side, y, "General", true, None)?;
            edit_general(
                canvas,
                view,
                local_view,
                [content[0], top, content[1], bottom],
            )
        }
        _ => {
            let y = side_preview(canvas, side, top)?;
            button(
                canvas,
                view,
                [side[0], y, side[1], y + canvas.r(5.6)],
                Variant::Hero,
                "Create",
                Some(local(A::Create)),
            )?;
            let mut y = y + canvas.r(5.6) + space(canvas, 2);
            for (label, tab) in [("General", Tab::General), ("Advanced", Tab::Advanced)] {
                let selected = local_view.tab == tab;
                y = tab_row(
                    canvas,
                    view,
                    side,
                    y,
                    label,
                    selected,
                    Some(local(A::Tab(tab))),
                )?;
            }
            let area = [content[0], top, content[1], bottom];
            match local_view.tab {
                Tab::General => create_general(canvas, view, local_view, area),
                Tab::Advanced => create_advanced(canvas, view, local_view, area),
            }
        }
    }
}

/// The 16:9 world preview at the top of the side menu; returns the y below it.
fn side_preview(
    canvas: &mut Canvas<'_>,
    side: [f32; 2],
    top: f32,
) -> Result<f32, UiPresentationError> {
    let preview = [
        side[0],
        top,
        side[1],
        top + (side[1] - side[0]) * 9.0 / 16.0,
    ];
    canvas.fill(preview, NEUTRAL100)?;
    canvas.frame(preview, EDGE, [0x1e, 0x1e, 0x1f, 255])?;
    Ok(preview[3] + space(canvas, 2))
}

/// One side-menu tab row; returns the y below it.
fn tab_row(
    canvas: &mut Canvas<'_>,
    view: &MenuView,
    side: [f32; 2],
    top: f32,
    label: &str,
    selected: bool,
    action: Option<MenuAction>,
) -> Result<f32, UiPresentationError> {
    let height = canvas.r(4.8);
    let b = [side[0], top, side[1], top + height];
    row(canvas, view, b, selected, action)?;
    canvas.text_line(
        label,
        [
            b[0] + canvas.r(1.6),
            top + (height - canvas.r(BODY.line)) * 0.5,
        ],
        b[2] - b[0] - canvas.r(3.2),
        BODY,
        TEXT,
    )?;
    Ok(b[3] + space(canvas, 1))
}

/// A control's label above it; returns the y below.
fn label(
    canvas: &mut Canvas<'_>,
    text: &str,
    area: Bounds,
    y: f32,
) -> Result<f32, UiPresentationError> {
    let height = canvas.text(text, [area[0], y], area[2] - area[0], BODY, TEXT, false)?;
    Ok(y + height + space(canvas, 1))
}

/// A description under a control; returns the y below plus the gap to the next control.
fn caption(
    canvas: &mut Canvas<'_>,
    text: &str,
    area: Bounds,
    y: f32,
) -> Result<f32, UiPresentationError> {
    let height = canvas.text(
        text,
        [area[0], y],
        area[2] - area[0],
        CAPTION,
        TEXT_DIMMEST,
        false,
    )?;
    Ok(y + height + space(canvas, 4))
}

fn name_field(
    canvas: &mut Canvas<'_>,
    view: &MenuView,
    name: &str,
    area: Bounds,
    y: f32,
) -> Result<f32, UiPresentationError> {
    let y = label(canvas, "World name", area, y)?;
    let focused = view.field == Some(MenuField::WorldName);
    let b = [area[0], y, area[2], y + canvas.r(FIELD)];
    text_field(
        canvas,
        view,
        b,
        name,
        "My World",
        focused,
        local(A::NameField),
    )?;
    let mut y = b[3] + space(canvas, 1);
    if let Some(error) = view.local.form_error {
        y += canvas.text(
            error,
            [area[0], y],
            area[2] - area[0],
            CAPTION,
            DESTRUCTIVE.fill,
            false,
        )?;
    }
    Ok(y + space(canvas, 3))
}

fn game_modes(
    canvas: &mut Canvas<'_>,
    view: &MenuView,
    modes: &[GameMode],
    current: GameMode,
    area: Bounds,
    y: f32,
) -> Result<f32, UiPresentationError> {
    let y = label(canvas, "Game mode", area, y)?;
    let options: Vec<_> = modes
        .iter()
        .map(|mode| {
            (
                game_mode_label(*mode),
                local(A::GameMode(*mode)),
                *mode == current,
            )
        })
        .collect();
    segmented(
        canvas,
        view,
        [area[0], y, area[2], y + canvas.r(5.2)],
        &options,
    )?;
    caption(
        canvas,
        game_mode_description(current),
        area,
        y + canvas.r(5.2) + space(canvas, 2),
    )
}

fn difficulties(
    canvas: &mut Canvas<'_>,
    view: &MenuView,
    current: Difficulty,
    area: Bounds,
    y: f32,
) -> Result<f32, UiPresentationError> {
    let y = label(canvas, "Difficulty", area, y)?;
    let options: Vec<_> = [
        Difficulty::Peaceful,
        Difficulty::Easy,
        Difficulty::Normal,
        Difficulty::Hard,
    ]
    .iter()
    .map(|d| {
        (
            difficulty_label(*d),
            local(A::Difficulty(*d)),
            *d == current,
        )
    })
    .collect();
    segmented(
        canvas,
        view,
        [area[0], y, area[2], y + canvas.r(5.2)],
        &options,
    )?;
    caption(
        canvas,
        difficulty_description(current),
        area,
        y + canvas.r(5.2) + space(canvas, 2),
    )
}

fn create_general(
    canvas: &mut Canvas<'_>,
    view: &MenuView,
    local_view: &WorldsView,
    area: Bounds,
) -> Result<(), UiPresentationError> {
    let form = &local_view.create;
    let y = name_field(canvas, view, &form.name, area, area[1])?;
    // New worlds offer Survival and Creative; Adventure appears when editing.
    let y = game_modes(
        canvas,
        view,
        &[GameMode::Survival, GameMode::Creative],
        form.game_mode,
        area,
        y,
    )?;
    difficulties(canvas, view, form.difficulty, area, y)?;
    Ok(())
}

fn create_advanced(
    canvas: &mut Canvas<'_>,
    view: &MenuView,
    local_view: &WorldsView,
    area: Bounds,
) -> Result<(), UiPresentationError> {
    let form = &local_view.create;
    let y = label(canvas, "World seed", area, area[1])?;
    let button_width = canvas.r(12.0).min((area[2] - area[0]) * 0.3);
    let field = [
        area[0],
        y,
        area[2] - button_width - space(canvas, 1),
        y + canvas.r(FIELD),
    ];
    let focused = view.field == Some(MenuField::WorldSeed);
    text_field(
        canvas,
        view,
        field,
        &form.seed_text,
        "3257840388504953787",
        focused,
        local(A::SeedField),
    )?;
    // Seed templates are a Marketplace list the core cannot serve.
    button(
        canvas,
        view,
        [field[2] + space(canvas, 1), y, area[2], field[3]],
        Variant::Secondary,
        "Templates",
        None,
    )?;
    let y = caption(
        canvas,
        "Guides the algorithm that magically creates your world",
        area,
        field[3] + space(canvas, 1),
    )?;
    let flat = form.generator == Generator::Flat;
    let switch_width = canvas.r(8.0);
    let text_width = area[2] - area[0] - switch_width - space(canvas, 2);
    let title_height = canvas.text("Flat world", [area[0], y], text_width, BODY, TEXT, false)?;
    let description = if local_view.bds_can_run {
        "A flat world to build up or mine down into"
    } else {
        "A flat world to build up or mine down into. Default worlds need Docker on this computer."
    };
    canvas.text(
        description,
        [area[0], y + title_height + space(canvas, 1)],
        text_width,
        CAPTION,
        TEXT_DIMMEST,
        false,
    )?;
    switch(
        canvas,
        view,
        [area[2] - switch_width, y, area[2], y + canvas.r(4.0)],
        flat,
        local(A::Flat(!flat)),
    )
}

fn edit_general(
    canvas: &mut Canvas<'_>,
    view: &MenuView,
    local_view: &WorldsView,
    area: Bounds,
) -> Result<(), UiPresentationError> {
    let Some(edit) = &local_view.edit else {
        return Ok(());
    };
    let y = name_field(canvas, view, &edit.name, area, area[1])?;
    let y = game_modes(
        canvas,
        view,
        &[GameMode::Survival, GameMode::Creative, GameMode::Adventure],
        edit.game_mode,
        area,
        y,
    )?;
    let y = difficulties(canvas, view, edit.difficulty, area, y)?;
    let y = label(canvas, "File management", area, y)?;
    let half = (area[2] - area[0] - space(canvas, 2)) * 0.5;
    button(
        canvas,
        view,
        [area[0], y, area[0] + half, y + canvas.r(CONTROL)],
        Variant::Destructive,
        "Delete world",
        Some(local(A::Delete)),
    )?;
    if let Some(world) = &local_view.edited {
        let details = format!(
            "Size: {} - Last saved: {}",
            crate::menu::file_size(world.size_bytes),
            crate::menu::civil_date(world.last_played_unix.max(world.created_unix)),
        );
        canvas.text(
            &details,
            [area[0], y + canvas.r(CONTROL) + space(canvas, 1)],
            area[2] - area[0],
            CAPTION,
            TEXT_DIMMEST,
            false,
        )?;
    }
    Ok(())
}

/// "Owned by me" with nothing owned: vanilla's no-content message and the Marketplace way out.
fn templates_empty(
    canvas: &mut Canvas<'_>,
    view: &MenuView,
    area: Bounds,
) -> Result<(), UiPresentationError> {
    let card = [area[0], area[1], area[2], area[1] + canvas.r(22.0)];
    panel(canvas, card)?;
    let pad = space(canvas, 4);
    let inner = [card[0] + pad, card[1] + pad, card[2] - pad, card[3] - pad];
    canvas.text_centred(
        "You don\u{2019}t own any content... yet!",
        [inner[0], inner[1], inner[2], inner[1] + canvas.r(2.4)],
        SECONDARY_BUTTON,
        TEXT,
        false,
    )?;
    let body = "Explore thousands of original worlds, templates, and skins on Marketplace \u{2014} or import your own.";
    canvas.text(
        body,
        [inner[0], inner[1] + canvas.r(3.6)],
        inner[2] - inner[0],
        CAPTION,
        TEXT_DIMMER,
        false,
    )?;
    let width = canvas.r(24.0).min(inner[2] - inner[0]);
    let left = (inner[0] + inner[2] - width) * 0.5;
    button(
        canvas,
        view,
        [left, inner[3] - canvas.r(CONTROL), left + width, inner[3]],
        Variant::Primary,
        "Go to Marketplace",
        Some(MenuAction::Store(crate::store::OPEN)),
    )
}
