use std::sync::Arc;

use valentine::bedrock::version::v1_26_51::{
    EnumsSetTitlePacketPayloadTitleType, EnumsTextPacketType, SetTitlePacket, TextPacket,
    TextPacketBody, TextPacketPayloadAuthorAndMessageMessageType as AuthoredType,
    TextPacketPayloadMessageAndParamsMessageType as ParameterType,
};

use super::{MAX_CHAT_PARAMETERS, UiEvent, UiPacketError, bounded_text};

/// Which of the three Text payload shapes the packet carried.
///
/// The leading union tag of `TextPacketBody`; the message type follows inside
/// the payload. gophertunnel derives the same byte from the message type
/// (`minecraft/protocol/packet/text.go`, `Text.Marshal`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextCategory {
    MessageOnly,
    Authored,
    Parameters,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextKind {
    Raw,
    Chat,
    Translation,
    Popup,
    JukeboxPopup,
    Tip,
    System,
    Whisper,
    Announcement,
    JsonWhisper,
    Json,
    JsonAnnouncement,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextEvent {
    pub category: TextCategory,
    pub kind: TextKind,
    pub needs_translation: bool,
    pub source: Option<Arc<str>>,
    pub message: Arc<str>,
    pub parameters: Arc<[Arc<str>]>,
    pub xuid: Arc<str>,
    pub platform_chat_id: Arc<str>,
    pub filtered_message: Option<Arc<str>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawTextEvent {
    pub text: TextEvent,
    pub document: Arc<crate::RawTextDocument>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TitleAction {
    Clear,
    Reset,
    SetTitle,
    SetSubtitle,
    ActionBar,
    SetDurations,
    SetTitleJson,
    SetSubtitleJson,
    ActionBarJson,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TitleEvent {
    pub action: TitleAction,
    pub text: Arc<str>,
    pub document: Option<Arc<crate::RawTextDocument>>,
    pub fade_in_ticks: i32,
    pub stay_ticks: i32,
    pub fade_out_ticks: i32,
    pub xuid: Arc<str>,
    pub platform_online_id: Arc<str>,
    pub filtered_message: Arc<str>,
}

pub(crate) fn normalize_text(packet: TextPacket) -> Result<UiEvent, UiPacketError> {
    let (category, kind, source, message, raw_text, parameters) = match packet.body {
        TextPacketBody::MessageOnly(payload) => match payload.message_type {
            EnumsTextPacketType::Raw => normalize_message_only(TextKind::Raw, payload.message)?,
            EnumsTextPacketType::Tip => normalize_message_only(TextKind::Tip, payload.message)?,
            EnumsTextPacketType::Systemmessage => {
                normalize_message_only(TextKind::System, payload.message)?
            }
            EnumsTextPacketType::Textobjectwhisper => {
                normalize_json_message(TextKind::JsonWhisper, payload.message)?
            }
            EnumsTextPacketType::Textobject => {
                normalize_json_message(TextKind::Json, payload.message)?
            }
            EnumsTextPacketType::Textobjectannouncement => {
                normalize_json_message(TextKind::JsonAnnouncement, payload.message)?
            }
            EnumsTextPacketType::Unknown(value) => return Err(unknown_text_type(value)),
        },
        TextPacketBody::AuthorAndMessage(payload) => {
            let kind = match payload.message_type {
                AuthoredType::Chat => TextKind::Chat,
                AuthoredType::Whisper => TextKind::Whisper,
                AuthoredType::Announcement => TextKind::Announcement,
                AuthoredType::Unknown(value) => return Err(unknown_text_type(value)),
            };
            normalize_authored(kind, payload.player_name, payload.message)?
        }
        TextPacketBody::MessageAndParams(payload) => {
            let kind = match payload.message_type {
                ParameterType::Translate => TextKind::Translation,
                ParameterType::Popup => TextKind::Popup,
                ParameterType::Jukeboxpopup => TextKind::JukeboxPopup,
                ParameterType::Unknown(value) => return Err(unknown_text_type(value)),
            };
            normalize_parameters(kind, payload.message, payload.parameter_list)?
        }
    };
    let event = TextEvent {
        category,
        kind,
        needs_translation: packet.localize,
        source,
        message,
        parameters,
        xuid: bounded_text(packet.senders_xuid)?,
        platform_chat_id: bounded_text(packet.platform_id)?,
        filtered_message: packet.filtered_message.map(bounded_text).transpose()?,
    };
    Ok(match raw_text {
        Some(document) => UiEvent::RawText(RawTextEvent {
            text: event,
            document,
        }),
        None => UiEvent::Text(event),
    })
}

fn unknown_text_type(value: u8) -> UiPacketError {
    UiPacketError::UnknownEnum {
        kind: "text type",
        value: i64::from(value),
    }
}

type NormalizedTextParts = (
    TextCategory,
    TextKind,
    Option<Arc<str>>,
    Arc<str>,
    Option<Arc<crate::RawTextDocument>>,
    Arc<[Arc<str>]>,
);

fn normalize_message_only(
    kind: TextKind,
    message: String,
) -> Result<NormalizedTextParts, UiPacketError> {
    let document = crate::raw_text::parse_raw_text_envelope(&message)?;
    let message = match &document {
        Some(document) => Arc::from(document.literal_text()),
        None => bounded_text(message)?,
    };
    Ok((
        TextCategory::MessageOnly,
        kind,
        None,
        message,
        document,
        Arc::from([]),
    ))
}

fn normalize_json_message(
    kind: TextKind,
    message: String,
) -> Result<NormalizedTextParts, UiPacketError> {
    let document = crate::parse_raw_text(&message)?;
    Ok((
        TextCategory::MessageOnly,
        kind,
        None,
        Arc::from(document.literal_text()),
        Some(document),
        Arc::from([]),
    ))
}

fn normalize_authored(
    kind: TextKind,
    player_name: String,
    message: String,
) -> Result<NormalizedTextParts, UiPacketError> {
    Ok((
        TextCategory::Authored,
        kind,
        Some(bounded_text(player_name)?),
        bounded_text(message)?,
        None,
        Arc::from([]),
    ))
}

fn normalize_parameters(
    kind: TextKind,
    message: String,
    parameter_list: Vec<String>,
) -> Result<NormalizedTextParts, UiPacketError> {
    if parameter_list.len() > MAX_CHAT_PARAMETERS {
        return Err(UiPacketError::TooManyChatParameters {
            count: parameter_list.len(),
            max: MAX_CHAT_PARAMETERS,
        });
    }
    let parameters = parameter_list
        .into_iter()
        .map(bounded_text)
        .collect::<Result<Vec<_>, _>>()?;
    Ok((
        TextCategory::Parameters,
        kind,
        None,
        bounded_text(message)?,
        None,
        Arc::from(parameters),
    ))
}

pub(crate) fn normalize_title(packet: SetTitlePacket) -> Result<UiEvent, UiPacketError> {
    // Wire values match gophertunnel's `TitleAction*` constants
    // (`minecraft/protocol/packet/set_title.go`); 1.26.40 renamed the generated
    // variants to the Mojang spellings.
    let action = match packet.title_type {
        EnumsSetTitlePacketPayloadTitleType::Clear => TitleAction::Clear,
        EnumsSetTitlePacketPayloadTitleType::Reset => TitleAction::Reset,
        EnumsSetTitlePacketPayloadTitleType::Title => TitleAction::SetTitle,
        EnumsSetTitlePacketPayloadTitleType::Subtitle => TitleAction::SetSubtitle,
        EnumsSetTitlePacketPayloadTitleType::Actionbar => TitleAction::ActionBar,
        EnumsSetTitlePacketPayloadTitleType::Times => TitleAction::SetDurations,
        EnumsSetTitlePacketPayloadTitleType::Titletextobject => TitleAction::SetTitleJson,
        EnumsSetTitlePacketPayloadTitleType::Subtitletextobject => TitleAction::SetSubtitleJson,
        EnumsSetTitlePacketPayloadTitleType::Actionbartextobject => TitleAction::ActionBarJson,
        EnumsSetTitlePacketPayloadTitleType::Unknown(value) => {
            return Err(UiPacketError::UnknownEnum {
                kind: "title action",
                value: i64::from(value),
            });
        }
    };
    let (text, document) = if matches!(
        action,
        TitleAction::SetTitleJson | TitleAction::SetSubtitleJson | TitleAction::ActionBarJson
    ) {
        let document = crate::parse_raw_text(&packet.title_text)?;
        (Arc::from(document.literal_text()), Some(document))
    } else {
        (bounded_text(packet.title_text)?, None)
    };
    Ok(UiEvent::Title(TitleEvent {
        action,
        text,
        document,
        fade_in_ticks: packet.fade_in_time,
        stay_ticks: packet.stay_time,
        fade_out_ticks: packet.fade_out_time,
        xuid: bounded_text(packet.xuid)?,
        platform_online_id: bounded_text(packet.platform_online_id)?,
        filtered_message: bounded_text(packet.filtered_title_message)?,
    }))
}
