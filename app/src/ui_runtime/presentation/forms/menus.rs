//! Menus through the engine: each menu state opens its vanilla screen, and the
//! screen's pressed regions become the launcher's own hit targets, so the menu
//! state machine and its input path stay unchanged. States without a vanilla
//! screen, or a render that fails, fall back to the programmatic launcher.

use json_ui::{HitRegion, ViewState};
use ui::{UiNode, UiRect};

use super::super::menu_scroll::ScrollArea;
use super::super::{TextMetrics, UiPresentationError, UiPresentationRuntime, menu, rect};
use super::{engine, menu_screens};
use crate::menu::{MenuAction, MenuScreen, MenuView};
use crate::ui_runtime::{UiRuntime, forms::EngineFrame};

const MODAL_POPUP: &str = "popup_dialog.modal_dialog_popup";

/// A menu frame's hit targets and, for hover next frame, their region keys.
type MenuHits = (Vec<(MenuAction, UiRect)>, Vec<(MenuAction, String)>);

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
        self.gui_scale_drag_targets.clear();
        let Some(view) = self.menu_view.take() else {
            return Ok(Vec::new());
        };
        self.menu_scrolls.begin_frame(format!(
            "{:?}/{:?}/{}",
            view.screen, view.server_tab, view.settings_section
        ));
        self.menu_scrolls.set_areas(Vec::new());
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
        // Without the UI carrier the programmatic launcher draws every screen.
        if self.form_presentation.engine.is_none() {
            return Ok(None);
        }
        // Screens 26.30 draws with OreUI by default draw natively.
        let portrait = [
            &view.feeds.profile.picture_path,
            &view.feeds.home.persona_head,
        ]
        .into_iter()
        .find_map(|path| self.menu_artwork.refs.get(path).copied());
        if let Some(hits) =
            self.append_oreui_screen(view, nodes, next, metrics, [width, height], portrait)?
        {
            let popup = self.append_dialog(
                runtime,
                view,
                &ViewState::default(),
                nodes,
                next,
                metrics,
                [width, height],
            );
            let (hits, keys) = popup.unwrap_or((hits, Vec::new()));
            self.form_presentation.menu_keys = keys;
            return Ok(Some(hits));
        }
        let Some(renderer) = self.form_presentation.engine.as_deref() else {
            return Ok(None);
        };
        let translate = |key: &str| runtime.translation(key);
        let Some(screen) = menu_screens::screen_data(view, &translate) else {
            return Ok(None);
        };
        // Settings lays out for tens of milliseconds; do it while a screen that opens it idles.
        if matches!(view.screen, MenuScreen::Home | MenuScreen::Pause) {
            let mut settings = view.clone();
            settings.screen = MenuScreen::Settings;
            if let Some(prepared) = menu_screens::screen_data(&settings, &translate) {
                let px = metrics.scale.get() * super::super::FONT_DESIGN_PIXEL_TEXELS as f32;
                renderer.prepare(engine::screen_cache::Prepared {
                    reference: prepared.reference,
                    context: prepared.context,
                    data: prepared.data,
                    root: [f64::from(width / px), f64::from(height / px)],
                    px,
                    language: translate("menu.play"),
                    font: std::sync::Arc::clone(&self.font),
                    metrics,
                    translator: runtime.translator(),
                });
            }
        }
        // Last frame's region keys carry the launcher's hover/press/focus.
        let key_of = |action: Option<MenuAction>| {
            let action = action?;
            self.form_presentation
                .menu_keys
                .iter()
                .find(|(candidate, _)| *candidate == action)
                .map(|(_, key)| key.clone())
        };
        let scroll = self
            .menu_scrolls
            .offsets()
            .iter()
            .map(|(key, offset)| (key.clone(), f64::from(*offset)))
            .collect();
        let state = ViewState {
            scroll,
            hovered: key_of(view.hovered).or_else(|| key_of(view.focused_action)),
            pressed: key_of(view.pressed),
            focused: view.field.and_then(|field| {
                key_of(Some(match field {
                    crate::menu::MenuField::Name => MenuAction::AddName,
                    crate::menu::MenuField::Address => MenuAction::AddAddress,
                    // Drawn by the OreUI create and edit screens, not JSON-UI.
                    crate::menu::MenuField::WorldName | crate::menu::MenuField::WorldSeed => {
                        return None;
                    }
                }))
            }),
        };
        let rollback = (nodes.len(), *next);
        // A popup draws over its screen and alone takes the input, so only the last frame's regions count.
        let mut layers = vec![&screen];
        layers.extend(screen.overlay.as_deref());
        let mut drawn = None;
        let preview_view = std::cell::Cell::new(None);
        for layer in layers {
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
                preview_view: Some(&preview_view),
                pointer: None,
                images: Some(&self.menu_artwork.refs),
                // The gamerpic, else the rendered persona head.
                portrait: [
                    &view.feeds.profile.picture_path,
                    &view.feeds.home.persona_head,
                ]
                .into_iter()
                .find_map(|path| self.menu_artwork.refs.get(path).copied()),
                splash: renderer.splash(&translate),
                now: self.menu_seconds,
                ..engine::ScreenArt::default()
            };
            match renderer.render_screen(
                layer.reference,
                &layer.data,
                &layer.context,
                &state,
                art,
                inputs,
                out,
            ) {
                Ok(Some(frame)) => drawn = Some(frame),
                Ok(None) | Err(_) => {
                    nodes.truncate(rollback.0);
                    *next = rollback.1;
                    return Ok(None);
                }
            }
        }
        if let Some(view) = preview_view.get() {
            self.player_preview_view = view.quantized();
        }
        let Some(frame) = drawn else {
            return Ok(None);
        };
        let mut hits = Vec::new();
        let mut keys = Vec::new();
        let origin = [self.safe_area.left(), self.safe_area.top()];
        self.menu_scrolls.set_areas(scroll_areas(&frame, origin));
        for region in frame.hits.iter().filter(|region| region.enabled) {
            if let Some(actions) = menu_screens::slider_actions(view, region) {
                if region.control_name.as_deref() == Some("gui_scale") {
                    let mut track = region.clone();
                    track.clip = track.rect;
                    self.gui_scale_drag_targets.extend(
                        segments(&track, actions.len(), frame.scale, origin)
                            .into_iter()
                            .map(|(step, bounds)| (actions[step], bounds)),
                    );
                }
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
        if let Some(popup) =
            self.append_dialog(runtime, view, &state, nodes, next, metrics, [width, height])
        {
            (hits, keys) = popup;
        }
        self.form_presentation.menu_keys = keys;
        Ok(Some(hits))
    }
}

impl UiPresentationRuntime {
    /// The vanilla popup for `view`'s open dialog, drawn over its screen, with
    /// the only hit targets that then count.
    #[allow(clippy::too_many_arguments)]
    fn append_dialog(
        &mut self,
        runtime: &UiRuntime,
        view: &MenuView,
        state: &ViewState,
        nodes: &mut Vec<UiNode>,
        next: &mut u32,
        metrics: TextMetrics,
        [width, height]: [f32; 2],
    ) -> Option<MenuHits> {
        let dialog = view.dialog?;
        let renderer = self.form_presentation.engine.as_deref()?;
        let translate = |key: &str| runtime.translation(key);
        let (model, confirm) = menu_screens::dialog_model(view, dialog, &translate);
        let context = json_ui::form_context(&model, &menu_screens::retail_context());
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
            nodes,
            next,
            overlay: &[],
        };
        let popup = renderer
            .render_screen(
                MODAL_POPUP,
                &data,
                &context,
                state,
                engine::ScreenArt {
                    now: self.menu_seconds,
                    ..engine::ScreenArt::default()
                },
                inputs,
                out,
            )
            .ok()??;
        let origin = [self.safe_area.left(), self.safe_area.top()];
        let mut hits = Vec::new();
        let mut keys = Vec::new();
        for region in popup.hits.iter().filter(|region| region.enabled) {
            let action = match region.pressed.as_deref() {
                Some("popup_dialog.left_button") => confirm,
                Some(
                    "popup_dialog.rightcancel_button" | "popup_dialog.escape" | "button.menu_exit",
                ) => MenuAction::DismissDialog,
                _ => continue,
            };
            if let Some(bounds) = window_rect(region, popup.scale, origin) {
                hits.push((action, bounds));
                keys.push((action, region.key.clone()));
            }
        }
        Some((hits, keys))
    }
}

/// A region's clipped rect in window-logical pixels.
pub(super) fn window_rect(region: &HitRegion, scale: f32, origin: [f32; 2]) -> Option<UiRect> {
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

/// The frame's scroll views in window-logical pixels, offsets in virtual px.
fn scroll_areas(frame: &EngineFrame, origin: [f32; 2]) -> Vec<ScrollArea> {
    let window = |r: [f64; 4]| {
        let to = |value: f64, axis: usize| value as f32 * frame.scale + origin[axis];
        rect(
            to(r[0], 0),
            to(r[1], 1),
            to(r[0] + r[2], 0),
            to(r[1] + r[3], 1),
        )
        .ok()
    };
    frame
        .hits
        .iter()
        .filter(|region| region.kind == json_ui::HitKind::ScrollView)
        .filter_map(|region| {
            let metrics = frame.report.scrolls.get(&region.key)?;
            Some(ScrollArea {
                key: region.key.clone(),
                viewport: window_rect(region, frame.scale, origin)?,
                scale: frame.scale,
                offset: metrics.offset as f32,
                max: metrics.max_offset() as f32,
                speed: metrics.speed as f32,
                track: metrics.track.and_then(window),
                thumb: metrics.thumb.and_then(window),
            })
        })
        .collect()
}

/// Regions select the nearest slider anchor, including anchors at both ends
/// of the track. The end values therefore occupy half an interior interval.
fn segments(
    region: &HitRegion,
    steps: usize,
    scale: f32,
    origin: [f32; 2],
) -> Vec<(usize, UiRect)> {
    let interval = region.rect.w / steps.saturating_sub(1).max(1) as f64;
    (0..steps)
        .filter_map(|step| {
            let mut part = region.clone();
            let left = if step == 0 {
                0.0
            } else {
                (step as f64 - 0.5) * interval
            };
            let right = if step + 1 == steps {
                region.rect.w
            } else {
                (step as f64 + 0.5) * interval
            };
            part.rect.x = region.rect.x + left;
            part.rect.w = right - left;
            window_rect(&part, scale, origin).map(|bounds| (step, bounds))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use json_ui::{HitKind, RectOut};
    use ui::UiPoint;

    use super::*;

    #[test]
    fn gui_scale_slider_regions_choose_the_nearest_native_step() {
        let rect = RectOut {
            x: 10.0,
            y: 20.0,
            w: 100.0,
            h: 12.0,
        };
        let region = HitRegion {
            key: "gui_scale".into(),
            name: "gui_scale".into(),
            kind: HitKind::Slider,
            rect,
            clip: rect,
            layer: 0,
            order: 0,
            pressed: None,
            control_name: Some("gui_scale".into()),
            collection_index: None,
            collection: None,
            enabled: true,
            checked: None,
            max_length: None,
            group_index: None,
            renderer: None,
        };
        let hits = segments(&region, 3, 2.0, [5.0, 7.0]);
        for (track_x, expected) in [
            (0.0, 0),
            (24.0, 0),
            (25.0, 1),
            (30.0, 1),
            (70.0, 1),
            (75.0, 2),
            (99.0, 2),
        ] {
            let point = UiPoint::new(5.0 + 2.0 * (10.0 + track_x), 7.0 + 2.0 * 26.0).unwrap();
            let selected = hits
                .iter()
                .rev()
                .find(|(_, bounds)| bounds.contains(point))
                .map(|(step, _)| *step);
            assert_eq!(selected, Some(expected), "track position {track_x}");
        }
    }
}
