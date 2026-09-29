//! Server-form presentation: the vanilla JSON-UI templates through the engine
//! when the UI carrier is loaded, else the programmatic fallback dialog.
mod container_kinds;
mod containers;
mod engine;
mod fallback;
mod menu_screens;
mod menus;
mod model;
mod npc;
#[cfg(test)]
pub(crate) mod pack_harness;
mod pages;
mod server_pack;
mod sign_editor;
#[cfg(test)]
pub(crate) mod tests;

use super::{TextMetrics, UiPresentationError, UiPresentationRuntime, dynamic_textures};
use crate::ui_runtime::{LocalFormAction, ServerFormIdentity, UiRuntime, forms::EngineFrame};
use assets::RuntimeUiAssets;
pub(crate) use containers::engine_panel_contains;
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
}

impl UiPresentationRuntime {
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
        self.form_presentation.engine = Some(Box::new(engine::FormEngine::new(
            assets, catalog, first_page,
        )));
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
        engine.set_server_pack(&pack.ui_layers);
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

    /// The dynamic pages holding the server pack's UI textures.
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
        self.form_presentation = FormPresentation {
            engine,
            menu_keys,
            logged,
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
                match model::engine_model(&entry.model, state, &translate) {
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
