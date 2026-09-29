//! Menus through the engine: each menu state opens its vanilla screen, and the
//! screen's pressed regions become the launcher's own hit targets, so the menu
//! state machine and its input path stay unchanged. States without a vanilla
//! screen, or a render that fails, fall back to the programmatic launcher.

use json_ui::{HitRegion, ViewState};
use ui::{UiNode, UiNodeId, UiRect, UiVisual};

use super::super::{TextMetrics, UiPresentationError, UiPresentationRuntime, menu, rect};
use super::{engine, menu_screens, oreui_profile, panorama};
use crate::menu::{MenuAction, MenuScreen, MenuView};
use crate::ui_runtime::UiRuntime;

const MODAL_POPUP: &str = "popup_dialog.modal_dialog_popup";
/// Backdrop behind launcher screens when the carrier lacks the panorama.
const LAUNCHER_BACKDROP: [u8; 4] = [8, 10, 14, 255];

impl UiPresentationRuntime {
    /// Draw the visible menu and return its window-logical hit targets.
    pub(crate) fn append_menu(
        &mut self,
        runtime: &UiRuntime,
        nodes: &mut Vec<UiNode>,
        next: &mut u32,
        metrics: TextMetrics,
        width: f32,
        height: f32,
    ) -> Result<Vec<(MenuAction, UiRect)>, UiPresentationError> {
        let Some(view) = self.menu_view.take() else {
            return Ok(Vec::new());
        };
        let drawn = if view.visible {
            self.append_engine_menu(runtime, &view, nodes, next, metrics, width, height)
        } else {
            Ok(Some(Vec::new()))
        };
        let result = match drawn {
            Ok(Some(hits)) => Ok(hits),
            Ok(None) | Err(_) => menu::append_menu_nodes(
                &view,
                nodes,
                next,
                &mut self.layouts,
                &self.font,
                metrics,
                self.solid_texture_page,
                width,
                height,
                self.safe_area,
            ),
        };
        self.menu_view = Some(view);
        result
    }

    /// `Ok(None)` when the engine has no screen for this state.
    #[allow(clippy::too_many_arguments)]
    fn append_engine_menu(
        &mut self,
        runtime: &UiRuntime,
        view: &MenuView,
        nodes: &mut Vec<UiNode>,
        next: &mut u32,
        metrics: TextMetrics,
        width: f32,
        height: f32,
    ) -> Result<Option<Vec<(MenuAction, UiRect)>>, UiPresentationError> {
        let Some(renderer) = self.form_presentation.engine.as_deref() else {
            return Ok(None);
        };
        // Profile is an OreUI route in 26.30; it draws natively over the panorama.
        let profile = view.screen == MenuScreen::Profile
            && !view.connecting
            && view.disconnect_message.is_none()
            && view.dialog.is_none()
            && !matches!(
                view.auth_state,
                crate::menu::auth::AuthState::AwaitingCode { .. }
            );
        if profile {
            let portrait = [
                &view.feeds.profile.picture_path,
                &view.feeds.home.persona_head,
            ]
            .into_iter()
            .find_map(|path| self.menu_artwork.refs.get(path).copied());
            let inputs = oreui_profile::ProfileInputs {
                layouts: &mut self.layouts,
                font: &self.font,
                metrics,
                solid_page: self.solid_texture_page,
                portrait,
                size: [width, height],
            };
            let hits = oreui_profile::append(view, inputs, nodes, next)?;
            self.form_presentation.menu_keys.clear();
            return Ok(Some(hits));
        }
        let translate = |key: &str| runtime.translation(key);
        let Some(screen) = menu_screens::screen_data(view, &translate) else {
            return Ok(None);
        };
        // Last frame's region keys carry the launcher's hover/press/focus.
        let key_of = |action: Option<MenuAction>| {
            let action = action?;
            self.form_presentation
                .menu_keys
                .iter()
                .find(|(candidate, _)| *candidate == action)
                .map(|(_, key)| key.clone())
        };
        let state = ViewState {
            hovered: key_of(view.hovered).or_else(|| key_of(view.focused_action)),
            pressed: key_of(view.pressed),
            focused: view.field.and_then(|field| {
                key_of(Some(match field {
                    crate::menu::MenuField::Name => MenuAction::AddName,
                    crate::menu::MenuField::Address => MenuAction::AddAddress,
                }))
            }),
            ..ViewState::default()
        };
        let rollback = (nodes.len(), *next);
        // Launcher screens sit on the panorama pass; in-game ones over the world.
        if !matches!(view.screen, MenuScreen::Pause | MenuScreen::Death)
            && !panorama::carried(renderer.assets())
        {
            nodes.push(
                UiNode::new(UiNodeId::new(*next), None, rect(0.0, 0.0, width, height)?)
                    .with_visual(UiVisual::Solid {
                        texture_page: self.solid_texture_page,
                        color: LAUNCHER_BACKDROP,
                    }),
            );
            *next = next.saturating_add(1);
        }
        let inputs = engine::EngineInputs {
            layouts: &mut self.layouts,
            font: &self.font,
            metrics,
            solid_page: self.solid_texture_page,
            safe_area: self.safe_area,
            content: [width, height],
            translate: &translate,
        };
        let out = engine::EngineOutput {
            nodes: &mut *nodes,
            next: &mut *next,
            overlay: &[],
        };
        let art = engine::ScreenArt {
            icons: &[],
            preview: self.hud_frame.player_preview,
            pointer: None,
            images: Some(&self.menu_artwork.refs),
            // The gamerpic, else the rendered persona head.
            portrait: [
                &view.feeds.profile.picture_path,
                &view.feeds.home.persona_head,
            ]
            .into_iter()
            .find_map(|path| self.menu_artwork.refs.get(path).copied()),
        };
        let rendered = renderer.render_screen(
            screen.reference,
            &screen.data,
            &screen.context,
            &state,
            art,
            inputs,
            out,
        );
        let frame = match rendered {
            Ok(Some(frame)) => frame,
            Ok(None) | Err(_) => {
                nodes.truncate(rollback.0);
                *next = rollback.1;
                return Ok(None);
            }
        };
        let mut hits = Vec::new();
        let mut keys = Vec::new();
        for region in frame.hits.iter().filter(|region| region.enabled) {
            let origin = [self.safe_area.left(), self.safe_area.top()];
            if let Some(actions) = menu_screens::slider_actions(region) {
                for (step, bounds) in segments(region, actions.len(), frame.scale, origin) {
                    hits.push((actions[step], bounds));
                }
                continue;
            }
            let Some(action) = menu_screens::action_for(view, region) else {
                continue;
            };
            if let Some(bounds) = window_rect(region, frame.scale, origin) {
                hits.push((action, bounds));
                keys.push((action, region.key.clone()));
            }
        }
        // A launcher dialog opens the vanilla popup and takes over the input.
        if let Some(dialog) = view.dialog {
            let (model, confirm) = menu_screens::dialog_model(view, dialog, &translate);
            let context = json_ui::form_context(&model, &json_ui::Context::desktop());
            let data = json_ui::form_data_source(&model);
            let inputs = engine::EngineInputs {
                layouts: &mut self.layouts,
                font: &self.font,
                metrics,
                solid_page: self.solid_texture_page,
                safe_area: self.safe_area,
                content: [width, height],
                translate: &translate,
            };
            let out = engine::EngineOutput {
                nodes: &mut *nodes,
                next: &mut *next,
                overlay: &[],
            };
            if let Ok(Some(popup)) = renderer.render_screen(
                MODAL_POPUP,
                &data,
                &context,
                &state,
                engine::ScreenArt::default(),
                inputs,
                out,
            ) {
                let origin = [self.safe_area.left(), self.safe_area.top()];
                hits.clear();
                keys.clear();
                for region in popup.hits.iter().filter(|region| region.enabled) {
                    let action = match region.pressed.as_deref() {
                        Some("popup_dialog.left_button") => confirm,
                        Some(
                            "popup_dialog.rightcancel_button"
                            | "popup_dialog.escape"
                            | "button.menu_exit",
                        ) => MenuAction::DismissDialog,
                        _ => continue,
                    };
                    if let Some(bounds) = window_rect(region, popup.scale, origin) {
                        hits.push((action, bounds));
                        keys.push((action, region.key.clone()));
                    }
                }
            }
        }
        self.form_presentation.menu_keys = keys;
        Ok(Some(hits))
    }
}

/// A region's clipped rect in window-logical pixels.
fn window_rect(region: &HitRegion, scale: f32, origin: [f32; 2]) -> Option<UiRect> {
    let x0 = region.rect.x.max(region.clip.x);
    let y0 = region.rect.y.max(region.clip.y);
    let x1 = (region.rect.x + region.rect.w).min(region.clip.x + region.clip.w);
    let y1 = (region.rect.y + region.rect.h).min(region.clip.y + region.clip.h);
    if x1 <= x0 || y1 <= y0 {
        return None;
    }
    let to = |value: f64, axis: usize| value as f32 * scale + origin[axis];
    rect(to(x0, 0), to(y0, 1), to(x1, 0), to(y1, 1)).ok()
}

/// A slider split into `steps` equal hit rects, one per value.
fn segments(
    region: &HitRegion,
    steps: usize,
    scale: f32,
    origin: [f32; 2],
) -> Vec<(usize, UiRect)> {
    let width = region.rect.w / steps.max(1) as f64;
    (0..steps)
        .filter_map(|step| {
            let mut part = region.clone();
            part.rect.x = region.rect.x + width * step as f64;
            part.rect.w = width;
            window_rect(&part, scale, origin).map(|bounds| (step, bounds))
        })
        .collect()
}
