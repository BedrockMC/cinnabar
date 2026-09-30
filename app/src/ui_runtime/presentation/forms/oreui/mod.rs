//! The OreUI design system, drawn in our own code, and the screens 26.30 shows
//! with OreUI by default (`docs/oreui.md`). The dev-only local-originals mode
//! swaps in the install's icon and border sprites for side-by-side comparison.

mod death;
mod friends;
mod grid;
mod icons;
mod inbox;
mod paint;
mod play;
mod play_realms;
mod play_servers;
mod profile;
mod theme;
mod widgets;

use std::sync::Arc;

use render::{UiRenderTextureArray, UiTexturePage};
use ui::{UiNode, UiRect};

use paint::Canvas;
pub(crate) use paint::Originals;

use super::super::{TextMetrics, UiPresentationError, UiPresentationRuntime};
use crate::menu::{MenuAction, MenuScreen, MenuView, auth::AuthState};
use crate::ui_runtime::oreui_assets::{OREUI_PAGE_SIDE, OreUiImages};

/// Which look OreUI screens draw with.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) enum Look {
    #[default]
    Drawn,
    /// The install's sprites where the drawn look would approximate them.
    Originals,
}

impl UiPresentationRuntime {
    /// Packs the dev-mode originals into a texture page; `CINNABAR_OREUI_LOOK=drawn`
    /// keeps the drawn look selected for comparison.
    pub(crate) fn enable_oreui_originals(&mut self, images: OreUiImages) -> Result<(), String> {
        let page = UiTexturePage::owned([OREUI_PAGE_SIDE, OREUI_PAGE_SIDE], images.rgba.into())
            .map_err(|error| format!("{error:?}"))?;
        let dynamic_start = self.textures.dynamic_start();
        let first = u16::try_from(dynamic_start).map_err(|_| "texture page overflow".to_owned())?;
        let mut pages = self.textures.pages()[..dynamic_start].to_vec();
        pages.push(page);
        pages.extend_from_slice(&self.textures.pages()[dynamic_start..]);
        let textures = UiRenderTextureArray::with_source_identity(
            pages,
            dynamic_start + 1,
            self.textures.static_identity(),
        )
        .map_err(|error| format!("{error:?}"))?;
        self.textures = Arc::new(textures);
        self.preview_dirty = true;
        self.menu_artwork_dirty = true;
        self.rebuild_dynamic_textures();
        let drawn = std::env::var("CINNABAR_OREUI_LOOK").is_ok_and(|look| look == "drawn");
        self.form_presentation.oreui_look = if drawn { Look::Drawn } else { Look::Originals };
        self.form_presentation.oreui_originals = Some(Arc::new(Originals {
            page: first,
            sprites: images.sprites,
        }));
        Ok(())
    }

    /// Draws `view` as an OreUI screen when 26.30 shows it with OreUI by
    /// default; `Ok(None)` leaves it to JSON-UI.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn append_oreui_screen(
        &mut self,
        view: &MenuView,
        nodes: &mut Vec<UiNode>,
        next: &mut u32,
        metrics: TextMetrics,
        size: [f32; 2],
        portrait: Option<super::super::IconRef>,
    ) -> Result<Option<Vec<(MenuAction, UiRect)>>, UiPresentationError> {
        // A launcher dialog draws over the OreUI screen instead.
        let covered = view.connecting
            || view.disconnect_message.is_some()
            || matches!(view.auth_state, AuthState::AwaitingCode { .. });
        let screen = view.screen;
        if covered
            || !matches!(
                screen,
                MenuScreen::Death
                    | MenuScreen::Profile
                    | MenuScreen::Inbox
                    | MenuScreen::Friends
                    | MenuScreen::Play
                    | MenuScreen::Social
                    | MenuScreen::Servers
            )
        {
            return Ok(None);
        }
        let originals = self
            .form_presentation
            .oreui_originals
            .clone()
            .filter(|_| self.form_presentation.oreui_look == Look::Originals);
        let mut canvas = Canvas::new(
            nodes,
            next,
            &mut self.layouts,
            &self.font,
            metrics,
            self.solid_texture_page,
            originals.as_deref(),
        );
        match screen {
            MenuScreen::Death => death::draw(&mut canvas, view, size)?,
            MenuScreen::Profile => profile::draw(&mut canvas, view, size, portrait)?,
            MenuScreen::Inbox => inbox::draw(&mut canvas, view, size)?,
            MenuScreen::Play | MenuScreen::Social | MenuScreen::Servers => {
                play::draw(&mut canvas, view, size, &self.menu_artwork.refs)?
            }
            _ => friends::draw(&mut canvas, view, size)?,
        }
        Ok(Some(canvas.hits))
    }
}
