//! Server-form presentation: the vanilla JSON-UI templates through the engine
//! when the UI carrier is loaded, else the programmatic fallback dialog.
mod book_screen;
mod chat_screen;
mod container_data;
mod container_kinds;
mod containers;
mod engine;
mod fallback;
mod global_resources;
mod hud;
mod join_progress;
mod loading_screen;
#[cfg(test)]
mod menu_latency;
mod menu_screens;
mod menus;
mod mod_hud;
mod model;
mod npc;
mod oreui;
#[cfg(test)]
pub(crate) mod pack_harness;
mod pages;
mod panorama;
pub(crate) use panorama::{built_in_faces, launcher_view};
#[cfg(test)]
mod play_flow_snapshots;
mod play_screen;
mod recipe_book;
mod remote_images;
mod server_pack;
mod settings_defaults;
mod sign_editor;
#[cfg(test)]
pub(crate) mod snapshot;
mod start_feed;
#[cfg(test)]
pub(crate) mod tests;
mod textures;
mod toast_screen;

pub(crate) use chat_screen::ChatHit;
pub(crate) use container_data::observe_station_block;
pub(crate) use loading_screen::LoadingStage;
pub(crate) use oreui::BedHit;
pub(crate) use panorama::drive_menu_panorama;

use super::{TextMetrics, UiPresentationError, UiPresentationRuntime, dynamic_textures};
use crate::ui_runtime::{LocalFormAction, ServerFormIdentity, UiRuntime, forms::EngineFrame};
use assets::RuntimeUiAssets;
pub(crate) use containers::{engine_panel_contains, engine_screen_for};
pub(crate) use engine::hud_renderers;
pub(crate) use recipe_book::{recipe_book_hover, recipe_book_icons, recipe_book_shown};
pub(crate) use server_pack::ServerUiPack;
use std::sync::Arc;
use ui::{UiNode, UiPoint, UiRect};

#[derive(Default)]
pub(super) struct FormPresentation {
    identity: Option<ServerFormIdentity>,
    pub(super) hits: Vec<(LocalFormAction, UiRect)>,
    pub(super) offsets: Vec<usize>,
    scroll: usize,
    height: usize,
    pub(super) maximum: usize,
    row_height: usize,
    /// The engine frame drawn this build, when the engine drew the form.
    frame: Option<EngineFrame>,
    /// The JSON-UI engine; carried across the per-frame reset.
    engine: Option<Box<engine::FormEngine>>,
    /// The container screen the engine drew this build, with its cell layout.
    container: Option<(EngineFrame, containers::ScreenLayout)>,
    /// The engine menu's regions by action, for next frame's hover state.
    menu_keys: Vec<(crate::menu::MenuAction, String)>,
    /// The form whose render path was last logged, so each form logs once.
    logged: Option<ServerFormIdentity>,
    /// The engine HUD's cached screens; carried across the per-frame reset.
    hud: hud::HudScreens,
    mod_hud: Option<mod_hud::ModHud>,
    /// The last container screen's layout; carried across the per-frame reset.
    container_cache: Option<containers::ScreenCache>,
    /// The open chat's cached screen; carried across the per-frame reset.
    chat: chat_screen::ChatScreen,
    /// The bed screen's hits and pointer; carried across the per-frame reset.
    bed: oreui::BedScreen,
    /// The sign editor's cached screen; carried across the per-frame reset.
    sign: sign_editor::SignScreen,
    /// Dev-mode OreUI originals and the look OreUI screens draw with.
    oreui_originals: Option<Arc<oreui::Originals>>,
    oreui_look: oreui::Look,
}

impl UiPresentationRuntime {
    /// Shares immutable carrier definitions with the optional-pack reload worker.
    pub(crate) fn pack_catalog_base(&self) -> Option<Arc<json_ui::Catalog>> {
        self.form_presentation
            .engine
            .as_ref()
            .map(|engine| engine.pack_catalog_base())
    }

    /// Bind the compiled UI carrier: its atlas pages join the texture array and
    /// its catalog drives server forms. On failure the fallback dialog stays.
    pub(crate) fn enable_json_ui(&mut self, assets: Arc<RuntimeUiAssets>) -> Result<(), String> {
        let catalog = json_ui::Catalog::from_files(
            assets
                .ui_files()
                .iter()
                .map(|file| (&*file.path, &*file.bytes)),
        )
        .map_err(|error| format!("ui catalog: {error}"))?;
        let (textures, first_page) = pages::with_ui_pages(&self.textures, &assets)
            .map_err(|error| format!("ui atlas pages: {error}"))?;
        self.textures = Arc::new(textures);
        // Dynamic pages moved up; their references rebuild from the new start.
        self.preview_dirty = true;
        self.menu_artwork_dirty = true;
        self.rebuild_dynamic_textures();
        let mut engine = engine::FormEngine::new(assets, catalog, first_page);
        engine.textures.server_page =
            (self.textures.dynamic_start() + dynamic_textures::SERVER_UI_PAGE) as u16;
        self.form_presentation.engine = Some(Box::new(engine));
        self.hud_frame.engine_containers = true;
        Ok(())
    }

    /// Overlay a joined server's resource-pack UI: its `ui/*.json` merge over the
    /// vanilla catalog layer by layer and its `textures/**` images shadow the
    /// carrier's from reserved dynamic pages, so the static texture identity the
    /// renderer pins never changes. An empty pack restores vanilla.
    pub(crate) fn set_server_ui_pack(&mut self, pack: &ServerUiPack) {
        let first = self.textures.dynamic_start() + dynamic_textures::SERVER_UI_PAGE;
        let Some(engine) = self.form_presentation.engine.as_mut() else {
            return;
        };
        if let Some(catalog) = &pack.catalog {
            engine.install_pack_catalog(catalog.clone());
        } else {
            engine.set_server_pack(&pack.ui_layers);
        }
        let atlas =
            server_pack::ServerAtlas::new(&pack.textures, dynamic_textures::SERVER_UI_PAGES);
        bevy::log::info!(
            layers = pack.ui_layers.len(),
            ui_files = pack.ui_layers.iter().map(Vec::len).sum::<usize>(),
            textures = pack.textures.len(),
            "server resource-pack UI applied to the form engine"
        );
        engine.set_server_atlas(atlas, first as u16);
        self.sync_server_ui_pages();
    }

    /// Hands changed server atlas pages to the dynamic texture pages; runs
    /// after the frame's screens drew, before the frame publishes.
    pub(super) fn sync_server_ui_pages(&mut self) {
        let changed = self
            .form_presentation
            .engine
            .as_mut()
            .is_some_and(|engine| engine.take_server_pages().is_some());
        if changed {
            self.rebuild_dynamic_textures();
        }
    }

    /// Let forms draw vanilla images the UI carrier lacks: item textures from
    /// the item icon atlas already on the UI texture array, anything else read
    /// on demand from the local vanilla pack at `vanilla`.
    pub(crate) fn set_form_texture_fallbacks(
        &mut self,
        entities: &assets::RuntimeEntityAssets,
        vanilla: std::path::PathBuf,
    ) {
        let mut icons = std::collections::HashMap::new();
        for visual in entities.item_visuals() {
            let assets::ItemVisualDefinitionRoute::Sprite { texture } = visual.route else {
                continue;
            };
            let Some(source) = entities.sources().get(texture.source as usize) else {
                continue;
            };
            let path = source
                .path
                .rsplit_once('.')
                .map_or(&*source.path, |(stem, _)| stem);
            if let Some(icon) = self.item_icon(&visual.key.identifier, visual.key.metadata) {
                icons.entry(path.to_owned()).or_insert(icon);
            }
        }
        if let Some(engine) = self.form_presentation.engine.as_mut() {
            engine.textures.set_fallbacks(icons, vanilla);
        }
    }

    /// The dynamic pages holding the server pack's UI textures.
    /// Drawn engine textures too big for a server page, for the art pages.
    pub(super) fn oversized_ui_textures(&self) -> Vec<(String, Arc<[u8]>)> {
        self.form_presentation
            .engine
            .as_ref()
            .map_or_else(Vec::new, |engine| engine.textures.oversized())
    }

    pub(super) fn server_ui_pages(&self) -> &[render::UiTexturePage] {
        self.form_presentation
            .engine
            .as_ref()
            .map_or(&[], |engine| &engine.server_pages)
    }

    /// Applies the runtime's server UI pack when it changes identity.
    pub(super) fn observe_server_ui(&mut self, pack: Option<&Arc<ServerUiPack>>) {
        let Some(engine) = self.form_presentation.engine.as_mut() else {
            return;
        };
        if !engine.take_server_source(pack) {
            return;
        }
        match pack {
            Some(pack) => self.set_server_ui_pack(&Arc::clone(pack)),
            None => self.set_server_ui_pack(&ServerUiPack::default()),
        }
    }

    /// The engine frame for `identity`, when the engine drew that form.
    pub(crate) fn form_engine_frame(&self, identity: ServerFormIdentity) -> Option<&EngineFrame> {
        self.form_presentation
            .frame
            .as_ref()
            .filter(|frame| frame.identity == Some(identity))
    }

    pub(crate) fn form_button_count(&self, identity: ServerFormIdentity) -> Option<usize> {
        (self.form_presentation.identity == Some(identity))
            .then_some(self.form_presentation.offsets.len().saturating_sub(1))
    }
    pub(crate) fn form_button_visible(&self, identity: ServerFormIdentity, index: usize) -> bool {
        self.form_presentation.identity == Some(identity)
            && self
                .form_presentation
                .hits
                .iter()
                .any(|(action, _)| *action == LocalFormAction::SubmitButton(index as u32))
    }
    pub(crate) fn hit_test_form(
        &self,
        point: UiPoint,
    ) -> Option<(ServerFormIdentity, LocalFormAction)> {
        let identity = self.form_presentation.identity?;
        self.form_presentation
            .hits
            .iter()
            .rev()
            .find_map(|(action, bounds)| bounds.contains(point).then_some((identity, *action)))
    }
    pub(crate) fn form_focus_scroll(
        &self,
        identity: ServerFormIdentity,
        index: usize,
    ) -> Option<usize> {
        let state = &self.form_presentation;
        if state.identity != Some(identity) {
            return None;
        }
        let top = *state.offsets.get(index)?;
        let bottom = top.saturating_add(state.row_height);
        Some(
            if top < state.scroll {
                top
            } else if bottom > state.scroll + state.height {
                bottom.saturating_sub(state.height)
            } else {
                state.scroll
            }
            .min(state.maximum),
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn append_server_form(
        &mut self,
        runtime: &UiRuntime,
        nodes: &mut Vec<UiNode>,
        next: &mut u32,
        metrics: TextMetrics,
        width: f32,
        height: f32,
    ) -> Result<(), UiPresentationError> {
        let engine = self.form_presentation.engine.take();
        let previous_container = self.form_presentation.container.take();
        let menu_keys = std::mem::take(&mut self.form_presentation.menu_keys);
        let logged = self.form_presentation.logged;
        let hud = std::mem::take(&mut self.form_presentation.hud);
        let mod_hud = self.form_presentation.mod_hud.take();
        let container_cache = self.form_presentation.container_cache.take();
        let chat = std::mem::take(&mut self.form_presentation.chat);
        let bed = std::mem::take(&mut self.form_presentation.bed);
        let sign = std::mem::take(&mut self.form_presentation.sign);
        self.form_presentation = FormPresentation {
            engine,
            menu_keys,
            logged,
            hud,
            mod_hud,
            container_cache,
            chat,
            bed,
            sign,
            oreui_originals: self.form_presentation.oreui_originals.take(),
            oreui_look: self.form_presentation.oreui_look,
            ..FormPresentation::default()
        };
        // Server settings draw over the settings menu; other forms wait it out.
        let settings_form = runtime
            .server_forms()
            .active()
            .is_some_and(|entry| entry.kind == protocol::FormKind::ServerSettings);
        if self.menu_view.is_some() && !settings_form {
            return Ok(());
        }
        self.append_engine_container(
            runtime,
            previous_container.as_ref().map(|(frame, _)| frame),
            nodes,
            next,
            metrics,
            width,
            height,
        )?;
        let Some(entry) = runtime.server_forms().active() else {
            return Ok(());
        };
        if let protocol::ServerFormModel::NpcDialogue(npc) = &entry.model
            && self.append_npc_dialogue(
                runtime,
                npc,
                entry.identity,
                nodes,
                next,
                metrics,
                width,
                height,
            )?
        {
            return Ok(());
        }
        let reason = match self.form_presentation.engine.as_deref_mut() {
            None => "JSON-UI carrier not loaded".to_owned(),
            Some(renderer) => {
                let translate = |key: &str| runtime.translation(key);
                let state = runtime.server_forms().engine();
                let remote = renderer.textures.remote.clone();
                let images = |url: &str| remote.state(url);
                match model::engine_model(&entry.model, state, &translate, &images) {
                    None => "form kind has no engine template".to_owned(),
                    Some(form) => {
                        let rollback = (nodes.len(), *next);
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
                        let catalog = renderer.catalog_label();
                        match renderer.render(&form, &state.view, entry.identity, inputs, out) {
                            Ok(Some(frame)) => {
                                self.form_presentation.frame = Some(frame);
                                log_path(
                                    &mut self.form_presentation.logged,
                                    entry.identity,
                                    "engine",
                                    &catalog,
                                );
                                return Ok(());
                            }
                            // A missing template or a node the tree rejects falls
                            // back to the programmatic dialog, not a blank screen.
                            Ok(None) => {
                                nodes.truncate(rollback.0);
                                *next = rollback.1;
                                format!("template did not resolve ({catalog})")
                            }
                            Err(error) => {
                                nodes.truncate(rollback.0);
                                *next = rollback.1;
                                format!("engine output rejected: {error}")
                            }
                        }
                    }
                }
            }
        };
        log_path(
            &mut self.form_presentation.logged,
            entry.identity,
            "fallback",
            &reason,
        );
        self.append_fallback_form(runtime, nodes, next, metrics, width, height)
    }
}

/// Logs which path draws `identity`, once per form.
fn log_path(
    logged: &mut Option<ServerFormIdentity>,
    identity: ServerFormIdentity,
    path: &str,
    reason: &str,
) {
    if *logged != Some(identity) {
        *logged = Some(identity);
        bevy::log::info!(?identity, path, reason, "server form render path");
    }
}
