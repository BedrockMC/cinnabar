//! The OreUI design system, drawn in our own code, and the screens vanilla shows
//! with OreUI by default (`docs/oreui.md`). Installed icon and control artwork
//! is read at runtime.

mod accounts;
mod add_server;
mod bedtime;
#[cfg(test)]
mod dark_mode_tests;
mod death;
mod dressing_room;
mod exit;
mod focus;
mod friends;
mod grid;
mod home;
mod icons;
mod inbox;
mod loading;
mod modal;
mod motion;
mod paint;
mod pause;
mod play;
mod play_realms;
mod play_servers;
mod profile;
mod progress;
mod radio;
#[cfg(test)]
mod review_tests;
#[cfg(test)]
mod route_tests;
mod scroll_focus;
mod settings;
#[cfg(test)]
mod settings_tests;
mod sidebar;
mod theme;
mod transitions;
mod widgets;
mod world_settings;

use std::sync::Arc;

use render_model::{UiRenderTextureArray, UiTexturePage};
use ui::{UiNode, UiPoint, UiRect};

pub use bedtime::BedHit;
use paint::Canvas;
pub use paint::Originals;
pub(super) use transitions::Transitions;

pub(super) struct CharacterPreview {
    pub(super) control: paint::Bounds,
    pub(super) clip: paint::Bounds,
}

use super::super::{TextMetrics, UiPresentationError, UiPresentationRuntime};
use crate::menu::{MenuAction, MenuScreen, MenuView, auth::AuthState};
use crate::ui_runtime::oreui_assets::OreUiImages;

/// Which look OreUI screens draw with.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum Look {
    #[default]
    Drawn,
    /// The install's sprites where the drawn look would approximate them.
    Originals,
}

impl UiPresentationRuntime {
    pub(in super::super) fn append_oreui_motion(
        &mut self,
        nodes: &mut Vec<UiNode>,
        next: &mut u32,
        size: [f32; 2],
    ) -> Result<(), UiPresentationError> {
        self.form_presentation.oreui_transitions.effects.finish(
            nodes,
            next,
            self.menu_seconds,
            size,
        )
    }

    pub(in super::super) fn append_oreui_loading(
        &mut self,
        stage: super::loading_screen::LoadingStage,
        words: [&str; 2],
        nodes: &mut Vec<UiNode>,
        next: &mut u32,
        metrics: TextMetrics,
        size: [f32; 2],
    ) -> Result<(), UiPresentationError> {
        use super::loading_screen::LoadingStage;
        let originals = self
            .form_presentation
            .oreui_originals
            .as_deref()
            .filter(|_| self.form_presentation.oreui_look == Look::Originals);
        let title_artwork = self
            .form_presentation
            .engine
            .as_deref()
            .and_then(|engine| engine.menu_title(&self.menu_artwork.refs));
        let destination_icon = (stage == LoadingStage::ChangingDimension)
            .then(|| {
                crate::ui_runtime::oreui_assets::dimensions::destination(self.hud_frame.dimension)
                    .and_then(|art| self.item_icon(art.block, 0))
            })
            .flatten();
        let mut canvas = Canvas::new(
            nodes,
            next,
            &mut self.layouts,
            &self.font,
            metrics,
            self.solid_texture_page,
            originals,
        );
        canvas.appearance = theme::Appearance::from_dark(self.form_presentation.oreui_dark_mode);
        canvas.seconds = self.menu_seconds;
        canvas.title_artwork = title_artwork;
        canvas.destination_icon = destination_icon;
        canvas.artwork = Some(&self.menu_artwork.refs);
        self.form_presentation
            .oreui_transitions
            .begin_frame(None, false, self.menu_seconds);
        canvas.transitions = Some(&mut self.form_presentation.oreui_transitions);
        progress::draw(
            &mut canvas,
            None,
            size,
            &progress::Progress {
                title: words[0],
                detail: words[1],
                fraction: None,
                cancel: None,
                indicator: stage != LoadingStage::ChangingDimension,
                stage,
                destination: (stage == LoadingStage::ChangingDimension)
                    .then_some(self.hud_frame.dimension),
            },
        )
    }

    /// Registers the installed artwork as immutable static texture pages.
    pub fn enable_oreui_originals(&mut self, images: OreUiImages) -> Result<(), String> {
        if self
            .form_presentation
            .oreui_originals
            .as_ref()
            .is_some_and(|old| {
                old.images.pages.len() == images.pages.len()
                    && old
                        .images
                        .pages
                        .iter()
                        .zip(&images.pages)
                        .all(|(old, new)| {
                            old.dimensions == new.dimensions
                                && Arc::ptr_eq(&old.pixels, &new.pixels)
                        })
                    && old.images.sprites == images.sprites
                    && old.images.animations == images.animations
                    && old.images.loading_frames == images.loading_frames
            })
        {
            return Ok(());
        }
        let originals = images
            .pages
            .iter()
            .map(|page| {
                UiTexturePage::owned(page.dimensions, page.pixels.clone())
                    .map_err(|error| format!("{error:?}"))
            })
            .collect::<Result<Vec<_>, _>>()?;
        let dynamic_start = self.textures.dynamic_start();
        let start = self
            .form_presentation
            .oreui_originals
            .as_ref()
            .map_or(dynamic_start, |old| usize::from(old.page));
        let first = u16::try_from(start).map_err(|_| "texture page overflow".to_owned())?;
        let count = originals.len();
        let mut pages = self.textures.pages()[..start].to_vec();
        pages.extend(originals);
        pages.extend_from_slice(&self.textures.pages()[dynamic_start..]);
        let textures = UiRenderTextureArray::with_source_identity(
            pages,
            start + count,
            self.textures.static_identity(),
        )
        .map_err(|error| format!("{error:?}"))?;
        self.textures = Arc::new(textures);
        if let Some(engine) = self.form_presentation.engine.as_mut() {
            engine.textures.server_page = (self.textures.dynamic_start()
                + super::super::dynamic_textures::SERVER_UI_PAGE)
                as u16;
        }
        self.preview_dirty = true;
        self.menu_artwork_dirty = true;
        self.rebuild_dynamic_textures();
        self.form_presentation.oreui_look = Look::Originals;
        let masks = images
            .sprites
            .iter()
            .filter_map(|(key, sprite)| {
                key.strip_prefix("@mask/")
                    .map(|key| (key.to_owned(), *sprite))
            })
            .collect();
        self.form_presentation.oreui_originals = Some(Arc::new(Originals {
            page: first,
            images: images.clone(),
            masks,
            sprites: images.sprites,
            loading_frames: images.loading_frames,
            animations: images.animations,
        }));
        Ok(())
    }

    /// Prepares additional native art before drawing; unchanged requests retain texture pages.
    pub fn prepare_oreui_artwork(&mut self, keys: &[&str]) -> Result<(), String> {
        if self
            .form_presentation
            .oreui_originals
            .as_ref()
            .is_some_and(|old| keys.iter().all(|key| old.sprites.contains_key(*key)))
        {
            return Ok(());
        }
        let images = crate::ui_runtime::oreui_assets::load_optional_oreui_images()
            .ok_or("OreUI installed artwork is unavailable")?;
        let old_start = self
            .form_presentation
            .oreui_originals
            .as_ref()
            .map_or(self.textures.dynamic_start(), |old| usize::from(old.page));
        let resident = self.textures.pages()[..old_start]
            .iter()
            .chain(&self.textures.pages()[self.textures.dynamic_start()..])
            .map(|page| page.pixels().len())
            .sum::<usize>();
        let available = render_model::MAX_UI_TEXTURE_BYTES
            .checked_sub(resident)
            .ok_or("Resident UI textures exceed the texture budget")?;
        self.enable_oreui_originals(images.with_artwork_budget(keys, available)?)
    }

    /// Draws an owned OreUI route, including the owner's menu design extensions.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn append_oreui_screen(
        &mut self,
        view: &MenuView,
        nodes: &mut Vec<UiNode>,
        next: &mut u32,
        metrics: TextMetrics,
        size: [f32; 2],
        portrait: Option<super::super::IconRef>,
        translate: super::menu_screens::Translate<'_>,
    ) -> Result<Option<Vec<(MenuAction, UiRect)>>, UiPresentationError> {
        // A launcher dialog draws over the OreUI screen instead.
        let progress = view.connecting || view.local.progress.is_some();
        let covered = view.disconnect_message.is_some()
            || matches!(view.auth_state, AuthState::AwaitingCode { .. });
        let screen = view.screen;
        if covered
            || (!progress
                && !matches!(
                    screen,
                    MenuScreen::Home
                        | MenuScreen::Death
                        | MenuScreen::DressingRoom
                        | MenuScreen::Settings
                        | MenuScreen::Profile
                        | MenuScreen::Inbox
                        | MenuScreen::Friends
                        | MenuScreen::Play
                        | MenuScreen::Social
                        | MenuScreen::Servers
                        | MenuScreen::AddServer
                        | MenuScreen::Pause
                ))
        {
            return Ok(None);
        }
        let originals = self
            .form_presentation
            .oreui_originals
            .clone()
            .filter(|_| self.form_presentation.oreui_look == Look::Originals);
        self.menu_scrolls.configure_motion(
            view.settings_options.value("screen_animations") != 0,
            self.menu_seconds,
        );
        let offsets = self.menu_scrolls.offsets().clone();
        let title_artwork = if screen == MenuScreen::Home && !progress {
            self.menu_artwork
                .refs
                .get(super::super::menu_artwork::TITLE_KEY)
                .copied()
        } else if progress || screen == MenuScreen::Pause {
            self.form_presentation.engine.as_deref().map_or_else(
                || {
                    self.menu_artwork
                        .refs
                        .get(super::super::menu_artwork::TITLE_KEY)
                        .copied()
                },
                |engine| engine.menu_title(&self.menu_artwork.refs),
            )
        } else {
            None
        };
        let mut canvas = Canvas::new(
            nodes,
            next,
            &mut self.layouts,
            &self.font,
            metrics,
            self.solid_texture_page,
            originals.as_deref(),
        );
        self.form_presentation.oreui_dark_mode = view.settings_options.oreui_dark_mode();
        canvas.appearance = theme::Appearance::from_dark(self.form_presentation.oreui_dark_mode);
        canvas.offsets = offsets;
        canvas.artwork = Some(&self.menu_artwork.refs);
        canvas.title_artwork = title_artwork;
        canvas.seconds = self.menu_seconds;
        canvas.slider_tracks = std::mem::take(&mut self.form_presentation.oreui_slider_tracks);
        canvas.focus_targets = std::mem::take(&mut self.form_presentation.menu_focus_geometry);
        canvas.focus_landmarks = std::mem::take(&mut self.form_presentation.menu_focus_landmarks);
        canvas.slider_tracks.clear();
        canvas.focus_targets.clear();
        canvas.focus_landmarks.clear();
        self.form_presentation.oreui_transitions.begin_frame(
            view.settings_control_activation,
            view.settings_control_activation_navigation,
            self.menu_seconds,
        );
        canvas.transitions = Some(&mut self.form_presentation.oreui_transitions);
        let root_surface = motion::Surface::Screen(match screen {
            MenuScreen::Social | MenuScreen::Servers | MenuScreen::AddServer => MenuScreen::Play,
            screen => screen,
        });
        let root_entrance = canvas.begin_entrance(root_surface);
        let motion_rem = canvas.rem;
        let mut dressing_preview = None;
        let mut character_preview = None;
        if progress {
            canvas.capture_focus = true;
            progress::join(&mut canvas, view, size, translate)?;
            self.form_presentation.menu_focus = canvas
                .focus_hits
                .iter()
                .map(|(action, _)| *action)
                .collect();
        } else {
            match screen {
                MenuScreen::Home => {
                    canvas.capture_focus = true;
                    let rollback = (canvas.nodes.len(), *canvas.next);
                    for attempt in 0..2 {
                        character_preview =
                            home::draw(&mut canvas, view, size, portrait, translate)?;
                        if !scroll_focus::reveal(&mut canvas, &mut self.menu_scrolls, view)
                            || attempt == 1
                        {
                            break;
                        }
                        canvas.nodes.truncate(rollback.0);
                        *canvas.next = rollback.1;
                        canvas.hits.clear();
                        canvas.clear_focus_geometry();
                        canvas.scrolls.clear();
                    }
                    self.form_presentation.menu_focus = canvas
                        .focus_hits
                        .iter()
                        .map(|(action, _)| *action)
                        .collect();
                }
                MenuScreen::Pause => {
                    canvas.capture_focus = true;
                    character_preview = pause::draw(&mut canvas, view, size)?;
                    self.form_presentation.menu_focus = canvas
                        .focus_hits
                        .iter()
                        .map(|(action, _)| *action)
                        .collect();
                }
                MenuScreen::DressingRoom => {
                    canvas.capture_focus = true;
                    let rollback = (canvas.nodes.len(), *canvas.next);
                    for attempt in 0..2 {
                        dressing_preview = Some(dressing_room::draw(&mut canvas, view, size)?);
                        if !scroll_focus::reveal(&mut canvas, &mut self.menu_scrolls, view)
                            || attempt == 1
                        {
                            break;
                        }
                        canvas.nodes.truncate(rollback.0);
                        *canvas.next = rollback.1;
                        canvas.hits.clear();
                        canvas.clear_focus_geometry();
                        canvas.scrolls.clear();
                    }
                    self.form_presentation.menu_focus = canvas
                        .focus_hits
                        .iter()
                        .map(|(action, _)| *action)
                        .collect();
                }
                MenuScreen::Death => {
                    canvas.appearance =
                        theme::Appearance::from_dark(self.form_presentation.oreui_dark_mode);
                    canvas.bundle = theme::Bundle::Gameplay;
                    death::draw(&mut canvas, view, size)?
                }
                MenuScreen::Profile => {
                    profile::draw(&mut canvas, view, size, portrait, &self.menu_artwork.refs)?
                }
                MenuScreen::Inbox => {
                    canvas.capture_focus = true;
                    let rollback = (canvas.nodes.len(), *canvas.next);
                    for attempt in 0..2 {
                        inbox::draw(&mut canvas, view, size, translate)?;
                        if !scroll_focus::reveal(&mut canvas, &mut self.menu_scrolls, view)
                            || attempt == 1
                        {
                            break;
                        }
                        canvas.nodes.truncate(rollback.0);
                        *canvas.next = rollback.1;
                        canvas.hits.clear();
                        canvas.clear_focus_geometry();
                        canvas.scrolls.clear();
                    }
                    self.form_presentation.menu_focus = canvas
                        .focus_hits
                        .iter()
                        .map(|(action, _)| *action)
                        .collect();
                }
                MenuScreen::Settings => {
                    let section = super::menu_screens::SETTINGS_SECTIONS
                        .iter()
                        .find_map(|(key, index)| (*index == view.settings_section).then_some(*key))
                        .unwrap_or("accessibility_forced_index");
                    canvas
                        .transitions
                        .as_deref_mut()
                        .unwrap()
                        .begin_settings(settings::section_index(section));
                    let rollback = (canvas.nodes.len(), *canvas.next);
                    for attempt in 0..2 {
                        settings::draw(
                            &mut canvas,
                            view,
                            size,
                            translate,
                            self.menu_artwork
                                .refs
                                .get(&view.feeds.profile.picture_path)
                                .copied(),
                        )?;
                        let adjusted =
                            scroll_focus::reveal(&mut canvas, &mut self.menu_scrolls, view);
                        if !adjusted || attempt == 1 {
                            break;
                        }
                        canvas.nodes.truncate(rollback.0);
                        *canvas.next = rollback.1;
                        canvas.hits.clear();
                        canvas.clear_focus_geometry();
                        canvas.scrolls.clear();
                        canvas.slider_tracks.clear();
                    }
                    self.form_presentation.menu_focus = canvas
                        .focus_hits
                        .iter()
                        .map(|(action, _)| *action)
                        .collect();
                    self.settings_slider_drag_targets.clear();
                    self.form_presentation.oreui_settings_input = true;
                }
                MenuScreen::Play | MenuScreen::Social | MenuScreen::Servers => {
                    match world_settings::route(view.local.screen, &view.local) {
                        Some(route) => {
                            canvas.capture_focus = true;
                            let rollback = (canvas.nodes.len(), *canvas.next);
                            for attempt in 0..2 {
                                let entrance = canvas.begin_entrance(motion::Surface::World(route));
                                world_settings::draw(&mut canvas, view, size, route)?;
                                canvas.end_entrance(entrance, size)?;
                                if !scroll_focus::reveal(&mut canvas, &mut self.menu_scrolls, view)
                                    || attempt == 1
                                {
                                    break;
                                }
                                canvas.nodes.truncate(rollback.0);
                                *canvas.next = rollback.1;
                                canvas.hits.clear();
                                canvas.clear_focus_geometry();
                                canvas.spots.clear();
                                canvas.scrolls.clear();
                            }
                            self.form_presentation.menu_focus = canvas
                                .focus_hits
                                .iter()
                                .map(|(action, _)| *action)
                                .collect();
                        }
                        None => play::draw(&mut canvas, view, size, &self.menu_artwork.refs)?,
                    }
                    if let Some(dialog) = modal::local_world_modal(&view.local) {
                        modal::draw(&mut canvas, view, size, &dialog)?;
                    }
                }
                MenuScreen::AddServer => {
                    play::draw_tab(&mut canvas, view, size, &self.menu_artwork.refs, 2)?;
                    canvas.hits.clear();
                    canvas.clear_focus_geometry();
                    canvas.spots.clear();
                    canvas.scrolls.clear();
                    canvas.capture_focus = true;
                    let rollback = (canvas.nodes.len(), *canvas.next);
                    for attempt in 0..2 {
                        add_server::draw(&mut canvas, view, size)?;
                        if !scroll_focus::reveal(&mut canvas, &mut self.menu_scrolls, view)
                            || attempt == 1
                        {
                            break;
                        }
                        canvas.nodes.truncate(rollback.0);
                        *canvas.next = rollback.1;
                        canvas.hits.clear();
                        canvas.clear_focus_geometry();
                        canvas.spots.clear();
                        canvas.scrolls.clear();
                    }
                    self.form_presentation.menu_focus = canvas
                        .focus_hits
                        .iter()
                        .map(|(action, _)| *action)
                        .collect();
                }
                _ => friends::draw(&mut canvas, view, size)?,
            }
        }
        let (mut hits, scrolls, spots, slider_tracks, focus_targets, focus_landmarks) = (
            canvas.hits,
            canvas.scrolls,
            canvas.spots,
            canvas.slider_tracks,
            canvas.focus_targets,
            canvas.focus_landmarks,
        );
        self.form_presentation.oreui_slider_tracks = slider_tracks;
        self.form_presentation.menu_focus_geometry = focus_targets;
        self.form_presentation.menu_focus_landmarks = focus_landmarks;
        self.menu_scrolls.set_areas(scrolls);
        self.add_menu_text_spots(spots);
        if let Some(preview) = character_preview {
            self.append_menu_player_preview(
                nodes,
                next,
                metrics,
                preview.control,
                preview.clip,
                super::super::player_preview::MenuPreviewConfig::DRESSING_ROOM,
            )?;
        }
        if let Some(preview) = dressing_preview {
            self.menu_skin_thumbnail_indices = preview.visible_skins;
            self.menu_cape_thumbnail_indices = preview.visible_capes;
            self.append_menu_player_preview(
                nodes,
                next,
                metrics,
                preview.control,
                preview.clip,
                super::super::player_preview::MenuPreviewConfig {
                    starting_rotation: if view.dressing_room.section
                        == launcher::dressing_room::DressingRoomSection::Capes
                    {
                        210.0
                    } else {
                        30.0
                    },
                    ..super::super::player_preview::MenuPreviewConfig::DRESSING_ROOM
                },
            )?;
            paint::apply_entrance(nodes, root_entrance, motion_rem, size)?;
            if view.dressing_room.editor.is_some() {
                self.menu_preview.control = None;
                self.cancel_menu_player_preview_input();
                let (editor_hits, focus, targets, landmarks, spots) = {
                    let mut editor = Canvas::new(
                        nodes,
                        next,
                        &mut self.layouts,
                        &self.font,
                        metrics,
                        self.solid_texture_page,
                        originals.as_deref(),
                    );
                    editor.appearance =
                        theme::Appearance::from_dark(self.form_presentation.oreui_dark_mode);
                    editor.artwork = Some(&self.menu_artwork.refs);
                    editor.capture_focus = true;
                    editor.seconds = self.menu_seconds;
                    editor.surface = root_surface;
                    editor.transitions = Some(&mut self.form_presentation.oreui_transitions);
                    dressing_room::draw_editor(&mut editor, view, size)?;
                    (
                        editor.hits,
                        editor
                            .focus_hits
                            .iter()
                            .map(|(action, _)| *action)
                            .collect(),
                        editor.focus_targets,
                        editor.focus_landmarks,
                        editor.spots,
                    )
                };
                self.form_presentation.menu_focus = focus;
                self.form_presentation.menu_focus_geometry = targets;
                self.form_presentation.menu_focus_landmarks = landmarks;
                hits = editor_hits;
                self.add_menu_text_spots(spots);
                self.menu_scrolls.set_areas(Vec::new());
            }
        } else {
            paint::apply_entrance(nodes, root_entrance, motion_rem, size)?;
        }
        self.form_presentation.menu_sounds.clear();
        Ok(Some(hits))
    }
}

/// The bed screen's last hit rects (window-logical) and the tracked pointer.
#[derive(Default)]
pub(super) struct BedScreen {
    hits: Vec<(BedHit, UiRect)>,
    pointer: Option<UiPoint>,
}

impl UiPresentationRuntime {
    /// Draws the OreUI bed screen while the player lies in bed.
    pub(in super::super) fn append_bed_screen(
        &mut self,
        runtime: &crate::ui_runtime::UiRuntime,
        nodes: &mut Vec<UiNode>,
        next: &mut u32,
        metrics: TextMetrics,
        size: [f32; 2],
        now_millis: u64,
    ) -> Result<(), UiPresentationError> {
        let bed = &mut self.form_presentation.bed;
        let Some(elapsed) = self.hud_frame.sleep.asleep_for(now_millis) else {
            bed.hits.clear();
            return Ok(());
        };
        let hovered = bed.pointer.and_then(|point| {
            bed.hits
                .iter()
                .find_map(|(hit, bounds)| bounds.contains(point).then_some(*hit))
        });
        let state = bedtime::Bedtime {
            elapsed,
            // The local player is on the list too.
            remote_players: runtime.known_player_names().len() > 1,
            thunderstorm: self.hud_frame.thunderstorm,
            status: runtime.sleep_status(),
            hovered,
            pressed: None,
        };
        let mut canvas = Canvas::new(
            nodes,
            next,
            &mut self.layouts,
            &self.font,
            metrics,
            self.solid_texture_page,
            None,
        );
        canvas.bundle = theme::Bundle::Gameplay;
        let hits = bedtime::draw(&mut canvas, &state, size)?;
        let [left, top] = [self.safe_area.left(), self.safe_area.top()];
        self.form_presentation.bed.hits = hits
            .into_iter()
            .filter_map(|(hit, bounds)| {
                let min = bounds.min();
                let max = bounds.max();
                super::super::rect(min.x() + left, min.y() + top, max.x() + left, max.y() + top)
                    .ok()
                    .map(|bounds| (hit, bounds))
            })
            .collect();
        Ok(())
    }

    /// What a press at the window-logical `position` hits on the bed screen.
    pub fn hit_test_bed(&self, position: UiPoint) -> Option<BedHit> {
        self.form_presentation
            .bed
            .hits
            .iter()
            .find_map(|(hit, bounds)| bounds.contains(position).then_some(*hit))
    }

    /// Track the pointer for next frame's hover state.
    pub fn set_bed_pointer(&mut self, position: Option<UiPoint>) {
        self.form_presentation.bed.pointer = position;
    }
}

#[cfg(test)]
impl UiPresentationRuntime {
    /// The bed screen's hit rects from the last frame.
    pub fn bed_hits(&self) -> &[(BedHit, UiRect)] {
        &self.form_presentation.bed.hits
    }
}
