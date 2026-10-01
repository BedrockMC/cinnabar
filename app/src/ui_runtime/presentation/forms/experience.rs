//! Host-owned consent and status chrome, resolved from a private JSON-UI catalog.

use std::sync::Arc;
use json_ui::{Catalog, Context, DataSource, Scalar, ViewState};
use server_experience::trust::Choice;
use ui::UiNode;
use super::{engine::{EngineInputs, EngineOutput, ScreenArt}, hud::CachedScreen};
use super::super::{FONT_DESIGN_PIXEL_TEXELS, TextMetrics, UiPresentationRuntime};
use crate::ui_runtime::forms::EngineFrame;

const TEMPLATE: &[u8] = include_bytes!("experience.json");

pub(super) struct ExperienceChrome {
    text: String,
    prompt: bool,
    labels: String,
    failed: bool,
    catalog: Arc<Catalog>,
    screen: CachedScreen,
    frame: Option<EngineFrame>,
}

impl UiPresentationRuntime {
    /// Remote packs cannot replace this catalog or its button actions.
    pub(crate) fn set_experience_chrome(&mut self, text: Option<&str>, prompt: bool) -> Result<(), String> {
        let Some(text) = text else {
            self.form_presentation.experience = None;
            return Ok(());
        };
        if let Some(chrome) = self.form_presentation.experience.as_mut() {
            if chrome.prompt != prompt || chrome.text != text { chrome.frame = None; }
            chrome.text = text.to_owned();
            chrome.prompt = prompt;
            return Ok(());
        }
        let catalog = Catalog::from_files([
            ("ui/_global_variables.json", &b"{}"[..]),
            ("ui/_ui_defs.json", &br#"{"ui_defs":["ui/cinnabar_experience.json"]}"#[..]),
            ("ui/cinnabar_experience.json", TEMPLATE),
        ]).map_err(|error| error.to_string())?;
        self.form_presentation.experience = Some(ExperienceChrome {
            text: text.to_owned(), prompt, labels: String::new(), failed: false, catalog: Arc::new(catalog),
            screen: CachedScreen::default(), frame: None,
        });
        Ok(())
    }

    /// Keeps guest labels in a separate, explicitly untrusted area below the status.
    pub(crate) fn set_experience_labels(&mut self, labels: &str) {
        if let Some(chrome) = self.form_presentation.experience.as_mut() {
            chrome.labels = labels.to_owned();
        }
    }

    /// Keyboard approval is possible only after a consent frame reached presentation.
    pub(crate) fn experience_prompt_visible(&self) -> bool {
        self.form_presentation.experience.as_ref()
            .is_some_and(|chrome| chrome.prompt && !chrome.failed && chrome.frame.is_some())
    }

    /// Missing trusted chrome revokes remote code instead of running it invisibly.
    pub(crate) fn experience_chrome_failed(&self) -> bool {
        self.form_presentation.experience.as_ref().is_some_and(|chrome| chrome.failed)
    }

    /// Maps only hits from the last host-owned consent screen to local choices.
    pub(crate) fn experience_choice(&self, point: [f32; 2]) -> Option<Choice> {
        let chrome = self.form_presentation.experience.as_ref()?;
        if !chrome.prompt { return None; }
        let frame = chrome.frame.as_ref()?;
        let point = [f64::from((point[0] - frame.origin[0]) / frame.scale), f64::from((point[1] - frame.origin[1]) / frame.scale)];
        let region = frame.hits.iter().rev().find(|region| region.enabled && region.contains(point))?;
        match region.pressed.as_deref()? {
            "experience.once" => Some(Choice::Once),
            "experience.always" => Some(Choice::Always),
            "experience.never" => Some(Choice::Never),
            _ => None,
        }
    }

    /// Draws trusted chrome last, including when server UI or the HUD is hidden.
    pub(in super::super) fn append_experience_chrome(
        &mut self,
        nodes: &mut Vec<UiNode>,
        next: &mut u32,
        metrics: TextMetrics,
        content: [f32; 2],
    ) {
        let Some(chrome) = self.form_presentation.experience.as_mut() else { return; };
        let Some(renderer) = self.form_presentation.engine.as_deref() else {
            chrome.failed = true;
            return;
        };
        let mut data = DataSource::new();
        data.set_global("#experience_text", Scalar::Text(chrome.text.clone()));
        data.set_global("#experience_widgets", Scalar::Text(chrome.labels.clone()));
        let inputs = EngineInputs {
            layouts: &mut self.layouts, font: &self.font, metrics,
            solid_page: self.solid_texture_page, safe_area: self.safe_area, content,
            translate: &|_| None,
        };
        let rollback = (nodes.len(), *next);
        let out = EngineOutput { nodes, next, overlay: &[] };
        let px = metrics.scale.get() * FONT_DESIGN_PIXEL_TEXELS as f32;
        let reference = if chrome.prompt { "cinnabar_experience.consent" } else { "cinnabar_experience.indicator" };
        match renderer.draw(ScreenArt::default(), inputs, out, |env, root| {
            chrome.screen.render_with(reference, &chrome.catalog, &Context::default(), data, (root, px), env, &ViewState::default())
        }) {
            Ok(frame) => {
                chrome.failed = frame.is_none();
                chrome.frame = frame;
            }
            Err(error) => {
                nodes.truncate(rollback.0);
                *next = rollback.1;
                chrome.frame = None;
                chrome.failed = true;
                bevy::log::warn!(%error, "server experience chrome could not render");
            }
        }
    }
}
