//! Small presentation primitives shared by the retained HUD builder.

use std::{borrow::Cow, sync::Arc};

use ui::{ChatMessage, ChatMessageKind, HudViewRole, UiPoint, UiRect};

use super::{MAX_PRESENTED_TEXT_BYTES, UiPresentationError};

/// The presented text for one chat row. `Translation`/command-output rows
/// resolve their key against `translate` and splice their ordered parameters
/// (an unknown key presents verbatim, as the vanilla client does); a `Chat`
/// row with a source gains the `<name>` prefix via `chat.type.text`. Other
/// kinds present their message unchanged. Bedrock `§` codes in the result are
/// parsed later at layout time.
pub(super) fn resolve_chat_line<'a>(
    node: &'a ChatMessage,
    translate: impl Fn(&str) -> Option<Arc<str>>,
) -> Cow<'a, str> {
    match node.kind {
        ChatMessageKind::Translation => match translate(&node.message) {
            Some(template) => {
                let arguments = node
                    .parameters
                    .iter()
                    .map(|parameter| parameter.as_ref().to_owned())
                    .collect::<Vec<_>>();
                Cow::Owned(protocol::format_translation(&template, &arguments))
            }
            None => Cow::Borrowed(node.message.as_ref()),
        },
        ChatMessageKind::Chat => match node.source.as_deref() {
            Some(source) if !source.is_empty() => {
                let template = translate("chat.type.text").unwrap_or_else(|| Arc::from("<%s> %s"));
                Cow::Owned(protocol::format_translation(
                    &template,
                    &[source.to_owned(), node.message.as_ref().to_owned()],
                ))
            }
            _ => Cow::Borrowed(node.message.as_ref()),
        },
        _ => Cow::Borrowed(node.message.as_ref()),
    }
}

pub(super) fn bounded_visible_text(value: &str) -> &str {
    if value.len() <= MAX_PRESENTED_TEXT_BYTES {
        return value;
    }
    let mut end = MAX_PRESENTED_TEXT_BYTES;
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    &value[..end]
}

pub(super) fn hud_position(role: HudViewRole, ordinal: usize, width: f32, height: f32) -> [f32; 2] {
    match role {
        HudViewRole::Health => [12.0, (height - 42.0).max(0.0)],
        HudViewRole::Hunger => [(width - 180.0).max(0.0), (height - 42.0).max(0.0)],
        HudViewRole::Armor => [12.0, (height - 62.0).max(0.0)],
        HudViewRole::Air => [(width - 180.0).max(0.0), (height - 62.0).max(0.0)],
        HudViewRole::Title => [(width * 0.3).max(0.0), (height * 0.3).max(0.0)],
        HudViewRole::Subtitle => [(width * 0.3).max(0.0), (height * 0.3 + 24.0).max(0.0)],
        HudViewRole::ActionBar => [(width * 0.35).max(0.0), (height - 90.0).max(0.0)],
        HudViewRole::ToastTitle | HudViewRole::ToastMessage => {
            [(width - 320.0).max(0.0), 12.0 + ordinal as f32 * 18.0]
        }
    }
}

pub(super) fn rect(
    left: f32,
    top: f32,
    right: f32,
    bottom: f32,
) -> Result<UiRect, UiPresentationError> {
    UiRect::new(
        UiPoint::new(left, top).map_err(UiPresentationError::Geometry)?,
        UiPoint::new(right, bottom).map_err(UiPresentationError::Geometry)?,
    )
    .map_err(UiPresentationError::Geometry)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn message(
        kind: ChatMessageKind,
        source: Option<&str>,
        text: &str,
        parameters: &[&str],
    ) -> ChatMessage {
        ChatMessage {
            fifo_sequence: 0,
            received_millis: 0,
            kind,
            source: source.map(Arc::from),
            message: Arc::from(text),
            parameters: parameters.iter().map(|value| Arc::from(*value)).collect(),
        }
    }

    #[test]
    fn translation_row_resolves_key_and_splices_ordered_parameters() {
        let node = message(
            ChatMessageKind::Translation,
            None,
            "death.attack.player",
            &["Legolas", "Gimli"],
        );
        let resolved = resolve_chat_line(&node, |key| {
            (key == "death.attack.player").then(|| Arc::from("%1$s was slain by %2$s"))
        });
        assert_eq!(resolved, "Legolas was slain by Gimli");
    }

    #[test]
    fn unknown_translation_key_presents_verbatim() {
        let node = message(ChatMessageKind::Translation, None, "death.attack.void", &[]);
        let resolved = resolve_chat_line(&node, |_| None);
        assert_eq!(resolved, "death.attack.void");
    }

    #[test]
    fn chat_row_prepends_the_source_name() {
        let node = message(ChatMessageKind::Chat, Some("Steve"), "hello", &[]);
        let resolved = resolve_chat_line(&node, |_| None);
        assert_eq!(resolved, "<Steve> hello");
    }

    #[test]
    fn chat_row_without_a_source_is_unchanged() {
        let node = message(ChatMessageKind::Chat, None, "server broadcast", &[]);
        let resolved = resolve_chat_line(&node, |_| None);
        assert_eq!(resolved, "server broadcast");
    }

    #[test]
    fn system_row_is_never_translated() {
        let node = message(ChatMessageKind::System, None, "chat.type.text", &["x"]);
        let resolved = resolve_chat_line(&node, |_| Some(Arc::from("<%s> %s")));
        assert_eq!(resolved, "chat.type.text");
    }
}
