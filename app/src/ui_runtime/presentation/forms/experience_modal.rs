//! A client part's modal screen: its signed JSON-UI templates drawn through the engine over
//! gameplay. Templates resolve only against the vanilla catalog as the carrier ships it (no
//! server resource-pack layer) and the bundle's own files; textures come only from the bundle
//! and the vanilla pack. Cinnabar's trusted chrome is a separate catalog drawn afterwards, so
//! it stays on top and out of reach.

use std::sync::Arc;

use json_ui::{Catalog, CollectionItem, DataSource, Scalar, ViewState};
use server_experience::{manifest::template_root, screen};
use ui::UiNode;

use super::super::{FONT_DESIGN_PIXEL_TEXELS, TextMetrics, UiPresentationRuntime};
use super::{
    engine::{EngineInputs, EngineOutput, ScreenArt},
    hud::CachedScreen,
    textures::TextureSet,
};
use crate::ui_runtime::{UiRuntime, forms::EngineFrame};

/// The modal a client part asks for: its bundle, verified screen files and bound data.
pub(crate) struct ExperienceModal<'a> {
    pub(crate) bundle: &'a str,
    pub(crate) files: &'a Arc<screen::Files>,
    pub(crate) modal: &'a screen::Modal,
}

pub(super) struct ModalScreen {
    bundle: String,
    files: Arc<screen::Files>,
    /// Vanilla plus the bundle's templates, built on first draw; `Err` names a rejected file.
    catalog: Option<Result<Arc<Catalog>, String>>,
    textures: Option<TextureSet>,
    /// The modal atlas's page images last handed to the dynamic pages.
    pages: Vec<render::UiTexturePage>,
    template: Option<String>,
    revision: Option<u64>,
    data: Arc<DataSource>,
    screen: CachedScreen,
    view: ViewState,
    /// The pointer in virtual pixels; the view sees it only when a control follows it.
    pointer: Option<[f64; 2]>,
    frame: Option<EngineFrame>,
}

impl UiPresentationRuntime {
    /// Follows the client part's modal, closed or open; `None` (no running client part) drops
    /// its catalog and textures.
    pub(crate) fn set_experience_modal(&mut self, modal: Option<ExperienceModal<'_>>) {
        let slot = &mut self.form_presentation.experience_modal;
        let Some(modal) = modal else {
            *slot = None;
            return;
        };
        if slot.as_ref().is_none_or(|current| {
            current.bundle != modal.bundle || !Arc::ptr_eq(&current.files, modal.files)
        }) {
            *slot = Some(ModalScreen {
                bundle: modal.bundle.to_owned(),
                files: Arc::clone(modal.files),
                catalog: None,
                textures: None,
                pages: Vec::new(),
                template: None,
                revision: None,
                data: Arc::default(),
                screen: CachedScreen::default(),
                view: ViewState::default(),
                pointer: None,
                frame: None,
            });
        }
        let screen = slot.as_mut().expect("modal installed");
        if screen.template != modal.modal.template {
            screen.template.clone_from(&modal.modal.template);
            screen.view = ViewState::default();
            screen.frame = None;
        }
        if screen.revision != Some(modal.modal.revision) {
            screen.revision = Some(modal.modal.revision);
            screen.data = Arc::new(data_source(modal.modal));
        }
    }

    /// Why the modal's templates were refused, which ends the client part.
    pub(crate) fn experience_modal_failure(&self) -> Option<&str> {
        match &self.form_presentation.experience_modal.as_ref()?.catalog {
            Some(Err(error)) => Some(error),
            _ => None,
        }
    }

    /// Whether the client part has a screen open, drawn yet or not; Escape and
    /// `ui.close-screen` both close it.
    pub(super) fn experience_modal_open(&self) -> bool {
        self.form_presentation
            .experience_modal
            .as_ref()
            .is_some_and(|screen| screen.template.is_some())
    }

    /// Whether the last build drew the modal, which then owns pointer and keyboard.
    pub(crate) fn experience_modal_shown(&self) -> bool {
        self.form_presentation
            .experience_modal
            .as_ref()
            .is_some_and(|screen| screen.frame.is_some())
    }

    /// Lights the control under the pointer and remembers it for scrolling.
    pub(crate) fn hover_experience_modal(&mut self, point: Option<[f32; 2]>) {
        let Some(screen) = self.form_presentation.experience_modal.as_mut() else {
            return;
        };
        let Some(frame) = &screen.frame else {
            return;
        };
        let point = point.map(|point| virtual_point(frame, point));
        screen.pointer = point;
        screen.view.pointer = point.filter(|_| frame.report.tracks_pointer);
        screen.view.hovered = point.and_then(|point| {
            frame
                .hits
                .iter()
                .rev()
                .find(|region| region.enabled && region.pressed.is_some() && region.contains(point))
                .map(|region| region.key.clone())
        });
    }

    /// Scrolls the scroll view under the pointer by wheel `notches` (positive scrolls down).
    pub(crate) fn scroll_experience_modal(&mut self, notches: f64) {
        let Some(screen) = self.form_presentation.experience_modal.as_mut() else {
            return;
        };
        let (Some(frame), Some([x, y])) = (&screen.frame, screen.pointer) else {
            return;
        };
        if notches == 0.0 {
            return;
        }
        let under = frame.report.scrolls.iter().find(|(_, metrics)| {
            metrics
                .viewport_rect
                .is_some_and(|[left, top, width, height]| {
                    (left..=left + width).contains(&x) && (top..=top + height).contains(&y)
                })
        });
        if let Some((key, metrics)) = under {
            let offset =
                (metrics.offset + notches * metrics.speed).clamp(0.0, metrics.max_offset());
            screen.view.scroll.insert(key.clone(), offset);
        }
    }

    /// Tracks a left press and returns the control id and collection row of a press released
    /// over the control it began on, as vanilla buttons fire.
    pub(crate) fn press_experience_modal(
        &mut self,
        point: Option<[f32; 2]>,
        pressed: bool,
        released: bool,
    ) -> Option<(String, Option<usize>)> {
        let screen = self.form_presentation.experience_modal.as_mut()?;
        let frame = screen.frame.as_ref()?;
        let region =
            point.and_then(|point| {
                let point = virtual_point(frame, point);
                frame.hits.iter().rev().find(|region| {
                    region.enabled && region.pressed.is_some() && region.contains(point)
                })
            });
        if pressed {
            screen.view.pressed = region.map(|region| region.key.clone());
        }
        if !released {
            return None;
        }
        let held = screen.view.pressed.take()?;
        let region = region.filter(|region| region.key == held)?;
        Some((region.pressed.clone()?, region.collection_index))
    }

    /// Draws the modal over the gameplay scenes when nothing else holds the screen; trusted
    /// chrome draws after it.
    pub(in super::super) fn append_experience_modal(
        &mut self,
        runtime: &UiRuntime,
        nodes: &mut Vec<UiNode>,
        next: &mut u32,
        metrics: TextMetrics,
        content: [f32; 2],
        over_gameplay: bool,
    ) {
        let Some(screen) = self.form_presentation.experience_modal.as_mut() else {
            return;
        };
        screen.frame = None;
        let Some(renderer) = self.form_presentation.engine.as_deref() else {
            return;
        };
        let Some(template) = screen.template.clone().filter(|_| over_gameplay) else {
            return;
        };
        let catalog = screen
            .catalog
            .get_or_insert_with(|| modal_catalog(&renderer.pack_catalog_base(), &screen.files));
        let Ok(catalog) = catalog.clone() else {
            return;
        };
        let page =
            (self.textures.dynamic_start() + super::super::dynamic_textures::MODAL_UI_PAGE) as u16;
        let textures = screen
            .textures
            .get_or_insert_with(|| renderer.textures.confined(&screen.files.textures, page));
        let reference = format!(
            "{}.{}",
            screen.files.namespace,
            template_root(&template).unwrap_or_default()
        );
        let translate = |key: &str| runtime.translation(key);
        let inputs = EngineInputs {
            layouts: &mut self.layouts,
            font: &self.font,
            metrics,
            solid_page: self.solid_texture_page,
            safe_area: self.safe_area,
            content,
            translate: &translate,
            language: runtime.text_generation(),
        };
        let rollback = (nodes.len(), *next);
        let out = EngineOutput {
            nodes,
            next,
            overlay: &[],
        };
        let px = metrics.scale.get() * FONT_DESIGN_PIXEL_TEXELS as f32;
        let art = ScreenArt {
            view: Some(&screen.view),
            ..ScreenArt::default()
        };
        let data = Arc::clone(&screen.data);
        let result = renderer.draw_with(textures, art, inputs, out, |env, root| {
            screen.screen.render_shared_with(
                &reference,
                &catalog,
                renderer.context(),
                data,
                (root, px, runtime.text_generation()),
                env,
                &screen.view,
            )
        });
        match result {
            Ok(frame) => screen.frame = frame,
            Err(error) => {
                nodes.truncate(rollback.0);
                *next = rollback.1;
                bevy::log::warn!(%error, bundle = %screen.bundle, "client part screen could not render");
            }
        }
    }

    /// Copies the modal atlas's page images when they changed; `true` asks for a page rebuild.
    pub(super) fn refresh_experience_modal_pages(&mut self) -> bool {
        let Some(screen) = self.form_presentation.experience_modal.as_mut() else {
            return false;
        };
        let Some(atlas) = screen.textures.as_mut().map(TextureSet::atlas_mut) else {
            return false;
        };
        if !atlas.take_dirty() {
            return false;
        }
        screen.pages = atlas.images().to_vec();
        true
    }

    /// The modal atlas's pages, for the dynamic pages reserved to it.
    pub(in super::super) fn experience_modal_pages(&self) -> &[render::UiTexturePage] {
        self.form_presentation
            .experience_modal
            .as_ref()
            .map_or(&[], |screen| &screen.pages)
    }
}

/// Every collection row and screen value as the engine's bindings read them.
fn data_source(modal: &screen::Modal) -> DataSource {
    let mut data = DataSource::new();
    for (name, value) in &modal.values {
        data.set_global(name.clone(), scalar(value));
    }
    for (name, rows) in &modal.collections {
        let items = rows
            .iter()
            .map(|row| {
                row.iter()
                    .fold(CollectionItem::default(), |item, (name, value)| {
                        item.with(name.clone(), scalar(value))
                    })
            })
            .collect();
        data.set_collection(name.clone(), items);
    }
    data
}

fn scalar(value: &screen::Value) -> Scalar {
    match value {
        screen::Value::Bool(value) => Scalar::Bool(*value),
        screen::Value::Integer(value) => Scalar::Int(*value),
        screen::Value::Number(value) => Scalar::Num(*value),
        screen::Value::Text(value) => Scalar::Text(value.clone()),
        screen::Value::Numbers(values) => Scalar::Json(values.as_slice().into()),
    }
}

fn virtual_point(frame: &EngineFrame, point: [f32; 2]) -> [f64; 2] {
    [
        f64::from((point[0] - frame.origin[0]) / frame.scale),
        f64::from((point[1] - frame.origin[1]) / frame.scale),
    ]
}

/// The vanilla catalog with the bundle's templates added in their own namespace. A namespace
/// the vanilla pack already has, a reference to a control vanilla lacks, or a template without
/// its root control is refused.
fn modal_catalog(vanilla: &Catalog, files: &screen::Files) -> Result<Arc<Catalog>, String> {
    let mut catalog = vanilla.clone();
    let before = catalog.namespace_count();
    for (path, bytes) in &files.templates {
        let references = screen::validate_template(bytes, &files.namespace)
            .map_err(|error| format!("{path}: {error}"))?;
        if let Some((namespace, name)) = references
            .iter()
            .find(|(namespace, name)| vanilla.lookup(namespace, name).is_none())
        {
            return Err(format!(
                "{path}: {namespace}.{name} is neither vanilla nor the bundle's"
            ));
        }
        catalog.overlay_text(path, &String::from_utf8_lossy(bytes));
    }
    if !files.templates.is_empty() && catalog.namespace_count() != before + 1 {
        return Err(format!(
            "namespace {} belongs to the vanilla pack",
            files.namespace
        ));
    }
    if let Some(path) = files.templates.keys().find(|path| {
        template_root(path).is_none_or(|root| catalog.lookup(&files.namespace, root).is_none())
    }) {
        return Err(format!("{path}: no root control named after the file"));
    }
    Ok(Arc::new(catalog))
}

#[cfg(test)]
mod tests;
