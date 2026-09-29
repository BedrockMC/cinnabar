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
mod pages;
mod server_pack;
mod sign_editor;
#[cfg(test)]
mod tests;

use super::{TextMetrics, UiPresentationError, UiPresentationRuntime};
use crate::ui_runtime::{LocalFormAction, ServerFormIdentity, UiRuntime, forms::EngineFrame};
use assets::RuntimeUiAssets;
pub(crate) use containers::engine_panel_contains;
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

    /// Overlay a joined server's resource pack (pack-relative paths): its
    /// `ui/*.json` merge over the vanilla catalog and its `textures/**` images
    /// shadow the carrier's. An empty set restores vanilla.
    #[cfg_attr(
        not(test),
        allow(dead_code, reason = "fed once server resource-pack application lands")
    )]
    pub(crate) fn set_server_ui_pack(&mut self, files: &[(String, Vec<u8>)]) {
        let Some(engine) = self.form_presentation.engine.as_mut() else {
            return;
        };
        engine.set_server_pack(files);
        let packed = server_pack::pack(files, engine.page_side());
        let start = engine.server_page_start();
        let old = engine.server_pages();
        let dynamic_start = self.textures.dynamic_start();
        let mut pages = self.textures.pages()[..start].to_vec();
        let added = packed.pages.len();
        pages.extend(packed.pages);
        pages.extend_from_slice(&self.textures.pages()[start + old..]);
        let Ok(textures) = render::UiRenderTextureArray::with_source_identity(
            pages,
            dynamic_start - old + added,
            server_pack_identity(self.textures.static_identity(), files),
        ) else {
            engine.set_server_textures(Default::default(), old);
            return;
        };
        engine.set_server_textures(packed.textures, added);
        self.textures = Arc::new(textures);
        // Dynamic pages moved; their references rebuild from the new start.
        self.preview_dirty = true;
        self.menu_artwork_dirty = true;
        self.rebuild_dynamic_textures();
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
        self.form_presentation = FormPresentation {
            engine,
            menu_keys,
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
        if let Some(renderer) = self.form_presentation.engine.as_deref() {
            let translate = |key: &str| runtime.translation(key);
            let state = runtime.server_forms().engine();
            if let Some(form) = model::engine_model(&entry.model, state, &translate) {
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
                match renderer.render(&form, &state.view, entry.identity, inputs, out) {
                    Ok(Some(frame)) => {
                        self.form_presentation.frame = Some(frame);
                        return Ok(());
                    }
                    // A missing template or a node the tree rejects falls back
                    // to the programmatic dialog rather than a blank screen.
                    Ok(None) | Err(_) => {
                        nodes.truncate(rollback.0);
                        *next = rollback.1;
                    }
                }
            }
        }
        self.append_fallback_form(runtime, nodes, next, metrics, width, height)
    }
}

fn server_pack_identity(base: [u8; 32], files: &[(String, Vec<u8>)]) -> [u8; 32] {
    use sha2::{Digest, Sha256};
    let mut digest = Sha256::new();
    digest.update(b"ui-server-pack-v1");
    digest.update(base);
    for (path, bytes) in files {
        digest.update(path.as_bytes());
        digest.update(Sha256::digest(bytes));
    }
    digest.finalize().into()
}
