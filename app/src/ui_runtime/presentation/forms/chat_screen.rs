//! The open chat through vanilla `chat.chat_screen`: the history as the screen
//! controller's `messages_factory`, the edit box, command suggestions and usage
//! lines as the `auto_complete` collection, and the send and back buttons.
//! Editing, completion and sending stay with `UiRuntime`.

use std::sync::Arc;

use json_ui::{
    CollectionItem, DataSource, FactoryItem, HitKind, HitRegion, Scalar, ScrollMetrics, ViewState,
};
use serde_json::Value;
use ui::{UiNode, UiPoint, UiRect};

use super::super::{
    FONT_DESIGN_PIXEL_TEXELS, TextMetrics, UiPresentationError, UiPresentationRuntime,
    bounded_visible_text, resolve_chat_line,
};
use super::engine::{EngineInputs, EngineOutput, ScreenArt};
use super::hud::CachedScreen;
use super::menus::window_rect;
use crate::ui_runtime::UiRuntime;

pub(crate) const CHAT_SCREEN: &str = "chat.chat_screen";
/// The messages factory's `max_children_size`.
const MAX_MESSAGES: usize = 100;
/// The edit box caret's on and off time.
const CARET_BLINK_MILLIS: u64 = 500;
const MESSAGES_VIEW: &str = "messages_panel";

/// What a pointer press on the chat screen means.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ChatHit {
    Suggestion(usize),
    Send,
    Close,
}

/// The chat screen's cached layout and last frame's input geometry.
#[derive(Default)]
pub(super) struct ChatScreen {
    screen: CachedScreen,
    /// Window-logical hit rects and their layout keys, from the last frame.
    hits: Vec<(ChatHit, UiRect, String)>,
    edit_box: Option<String>,
    /// The messages view's key and extents from the last frame.
    scroll: Option<(String, ScrollMetrics)>,
    /// Virtual px scrolled up from the newest message.
    from_bottom: f64,
    messages: usize,
    open: bool,
    /// Logical px per virtual px, from the last frame.
    scale: f32,
    pointer: Option<UiPoint>,
}

impl UiPresentationRuntime {
    /// Draw the open chat; `Ok(false)` when the engine is not loaded.
    pub(in super::super) fn append_chat_screen(
        &mut self,
        runtime: &UiRuntime,
        nodes: &mut Vec<UiNode>,
        next: &mut u32,
        metrics: TextMetrics,
        content: [f32; 2],
        now_millis: u64,
    ) -> Result<bool, UiPresentationError> {
        let Some(renderer) = self.form_presentation.engine.as_deref() else {
            return Ok(false);
        };
        let chat = &mut self.form_presentation.chat;
        let messages = runtime.chat().messages().len();
        // The view jumps to the newest message on open and on every update.
        if !chat.open || messages != chat.messages {
            chat.from_bottom = 0.0;
        }
        chat.open = true;
        chat.messages = messages;
        let data = chat_data(runtime, now_millis);
        let view = chat.view_state(runtime.chat_selected_suggestion());
        let context = renderer.context().clone();
        let catalog = Arc::clone(renderer.catalog());
        let px = metrics.scale.get() * FONT_DESIGN_PIXEL_TEXELS as f32;
        let translate = |key: &str| runtime.translation(key);
        let inputs = EngineInputs {
            layouts: &mut self.layouts,
            font: &self.font,
            metrics,
            solid_page: self.solid_texture_page,
            safe_area: self.safe_area,
            content,
            translate: &translate,
        };
        let out = EngineOutput {
            nodes,
            next,
            overlay: &[],
        };
        let art = ScreenArt {
            now: now_millis as f64 / 1_000.0,
            ..ScreenArt::default()
        };
        let screen = &mut chat.screen;
        let frame = renderer.draw(art, inputs, out, |env, root| {
            screen.render_with(
                CHAT_SCREEN,
                &catalog,
                &context,
                data,
                (root, px),
                env,
                &view,
            )
        })?;
        chat.hits.clear();
        chat.edit_box = None;
        let Some(frame) = frame else {
            return Ok(true);
        };
        chat.scale = frame.scale;
        chat.scroll = frame
            .report
            .scrolls
            .iter()
            .find(|(key, _)| key.contains(MESSAGES_VIEW))
            .map(|(key, metrics)| (key.clone(), *metrics));
        for region in frame.hits.iter().filter(|region| region.enabled) {
            if region.kind == HitKind::EditBox {
                chat.edit_box = Some(region.key.clone());
                continue;
            }
            let Some(hit) = chat_hit(region) else {
                continue;
            };
            if let Some(bounds) = window_rect(region, frame.scale, frame.origin) {
                chat.hits.push((hit, bounds, region.key.clone()));
            }
        }
        Ok(true)
    }

    /// Forget the open chat's scroll and hover once the screen closes.
    pub(in super::super) fn close_chat_screen(&mut self) {
        let chat = &mut self.form_presentation.chat;
        chat.open = false;
        chat.hits.clear();
        chat.pointer = None;
    }

    /// What a press at the window-logical `position` hits on the open chat.
    pub(crate) fn hit_test_chat(&self, position: UiPoint) -> Option<ChatHit> {
        self.form_presentation
            .chat
            .hits
            .iter()
            .rev()
            .find_map(|(hit, bounds, _)| bounds.contains(position).then_some(*hit))
    }

    /// Track the pointer for next frame's hover state.
    pub(crate) fn set_chat_pointer(&mut self, position: Option<UiPoint>) {
        self.form_presentation.chat.pointer = position;
    }

    /// Scroll the history by a wheel delta in notches, or logical px when
    /// `pixels`; positive scrolls toward older messages.
    pub(crate) fn scroll_chat(&mut self, delta: f32, pixels: bool) {
        let chat = &mut self.form_presentation.chat;
        let Some((_, metrics)) = chat.scroll else {
            return;
        };
        let step = if pixels {
            f64::from(delta / chat.scale.max(f32::EPSILON))
        } else {
            f64::from(delta) * metrics.speed
        };
        chat.from_bottom = (chat.from_bottom + step).clamp(0.0, metrics.max_offset());
    }
}

impl ChatScreen {
    fn view_state(&self, selected: Option<usize>) -> ViewState {
        let hovered = self
            .pointer
            .and_then(|point| {
                self.hits
                    .iter()
                    .rev()
                    .find(|(_, bounds, _)| bounds.contains(point))
            })
            .or_else(|| {
                // The keyboard-selected suggestion shows its focus border.
                let selected = selected?;
                self.hits
                    .iter()
                    .find(|(hit, ..)| *hit == ChatHit::Suggestion(selected))
            })
            .map(|(_, _, key)| key.clone());
        let mut view = ViewState {
            hovered,
            focused: self.edit_box.clone(),
            ..ViewState::default()
        };
        // Unscrolled, the view's `jump_to_bottom_on_update` keeps it on the newest line.
        if let Some((key, metrics)) = &self.scroll
            && self.from_bottom > 0.0
        {
            view.scroll.insert(
                key.clone(),
                (metrics.max_offset() - self.from_bottom).max(0.0),
            );
        }
        view
    }
}

/// What the chat controller binds, from the runtime's chat state.
fn chat_data(runtime: &UiRuntime, now_millis: u64) -> DataSource {
    let mut data = DataSource::new();
    data.set_strict(true);
    let translate = |key: &str| runtime.translation(key);
    let text = |key: &str, fallback: &str| {
        Scalar::Text(translate(key).map_or_else(|| fallback.to_owned(), |text| text.to_string()))
    };
    let editor = runtime.chat_editor();
    let mut content = String::with_capacity(editor.as_str().len() + 1);
    content.push_str(&editor.as_str()[..editor.cursor_byte()]);
    if (now_millis / CARET_BLINK_MILLIS).is_multiple_of(2) {
        content.push('|');
    }
    content.push_str(&editor.as_str()[editor.cursor_byte()..]);
    data.set_global("#message_text_box_content", Scalar::Text(content));
    for flag in ["#send_button_visible", "#chat_title_visible"] {
        data.set_global(flag, Scalar::Bool(true));
    }
    data.set_global("#chat_title_text", text("chat.title", "Chat"));
    data.set_global(
        "#back_button_text",
        text("controller.buttonTip.back", "Back"),
    );
    data.set_global(
        "#send_button_accessibility_text",
        text("accessibility.chat.tts.send", ""),
    );
    let messages = runtime.chat().messages();
    let items = messages
        .iter()
        .skip(messages.len().saturating_sub(MAX_MESSAGES))
        .map(|line| {
            let text = resolve_chat_line(line, translate);
            FactoryItem::new("chat_screen_messages", 0.0)
                .value(
                    "#text",
                    Scalar::Text(bounded_visible_text(text.as_ref()).to_owned()),
                )
                .var("chat_font_type", Value::from("default"))
                .var("chat_font_scale_factor", Value::from(1.0))
                .var("chat_line_spacing", Value::from(0.0))
        })
        .collect();
    data.set_factory("messages_factory", items);
    let row = |text: &str, suggestion: bool| {
        CollectionItem::default()
            .with(
                "#auto_complete_text",
                Scalar::Text(bounded_visible_text(text).to_owned()),
            )
            .with("#is_autocomplete_suggestion", Scalar::Bool(suggestion))
    };
    // Suggestions list upward from the edit box; the usage line sits nearest it.
    let mut rows: Vec<CollectionItem> = runtime
        .chat_suggestions()
        .iter()
        .map(|suggestion| row(suggestion, true))
        .collect();
    rows.extend(runtime.chat_usage_hint().map(|usage| row(usage, false)));
    // The history's `#chat_visible` gives way to the suggestion list.
    data.set_global("#chat_visible", Scalar::Bool(rows.is_empty()));
    data.set_collection("auto_complete", rows);
    data
}

fn chat_hit(region: &HitRegion) -> Option<ChatHit> {
    match region.pressed.as_deref()? {
        "button.click_autocomplete" => region.collection_index.map(ChatHit::Suggestion),
        "button.send" => Some(ChatHit::Send),
        "button.menu_exit" | "button.menu_cancel" | "button.chat_menu_cancel" => {
            Some(ChatHit::Close)
        }
        _ => None,
    }
}

#[cfg(test)]
impl UiPresentationRuntime {
    /// The open chat's last laid-out draw nodes, in virtual px.
    pub(crate) fn chat_draw_nodes(&self) -> &[json_ui::DrawNode] {
        self.form_presentation.chat.screen.nodes()
    }

    /// The open chat's hit rects from the last frame.
    pub(crate) fn chat_hits(&self) -> Vec<(ChatHit, UiRect)> {
        let hits = &self.form_presentation.chat.hits;
        hits.iter()
            .map(|(hit, bounds, _)| (*hit, *bounds))
            .collect()
    }
}
