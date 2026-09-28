//! Server resource-pack sound overrides: definitions, `sounds.json` routing and decodable files.
//!
//! The pack stack is prepared off the frame loop, so the result crosses through a generation-stamped
//! mailbox the engine polls each frame.

use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use assets::{AudioAlternative, AudioDefinition, SoundEventTables, decode_fsb5};
use resource_pack::LayeredPackView;
use serde_json::{Map, Value};

use super::voice::Pcm;

const MAX_FILE_BYTES: u64 = 16 * 1024 * 1024;
const MAX_TOTAL_PCM_BYTES: usize = 64 * 1024 * 1024;
const MAX_DEFINITIONS: usize = 8192;

pub(crate) struct ServerSoundPack {
    pub definitions: HashMap<Box<str>, AudioDefinition>,
    pub tables: Option<SoundEventTables>,
    pub files: HashMap<Box<str>, Arc<Pcm>>,
}

fn float(value: Option<&Value>) -> Option<f32> {
    value?.as_f64().map(|n| n as f32).filter(|n| n.is_finite())
}

fn alternative(value: &Value) -> Option<AudioAlternative> {
    let (name, object) = match value {
        Value::String(name) => (name.as_str(), None),
        Value::Object(map) => (map.get("name")?.as_str()?, Some(map)),
        _ => return None,
    };
    let flag = |key: &str| object?.get(key)?.as_bool();
    Some(AudioAlternative {
        object_form: object.is_some(),
        name: name.into(),
        weight: object
            .and_then(|map| map.get("weight")?.as_u64())
            .map_or(1, |weight| weight.clamp(1, u64::from(u16::MAX)) as u16),
        volume: float(object.and_then(|map| map.get("volume"))),
        pitch: float(object.and_then(|map| map.get("pitch"))),
        is_3d: flag("is3D"),
        stream: flag("stream"),
        load_on_low_memory: flag("load_on_low_memory"),
    })
}

fn definition(name: &str, value: &Value) -> Option<AudioDefinition> {
    let map = value.as_object()?;
    let alternatives: Vec<AudioAlternative> = map
        .get("sounds")?
        .as_array()?
        .iter()
        .filter_map(alternative)
        .collect();
    let text = |key: &str| map.get(key).and_then(Value::as_str).map(Box::<str>::from);
    Some(AudioDefinition {
        identifier: name.into(),
        category: text("category"),
        subtitle: text("subtitle"),
        min_distance: float(map.get("min_distance")),
        max_distance: float(map.get("max_distance")),
        volume: float(map.get("volume")),
        pitch: float(map.get("pitch")),
        use_legacy_max_distance: map.get("__use_legacy_max_distance").map(|value| {
            value
                .as_str()
                .map_or_else(|| value.to_string(), str::to_owned)
                .into()
        }),
        alternatives: alternatives.into_boxed_slice(),
    })
}

/// 16-bit PCM RIFF/WAVE, mono or stereo; anything else is skipped.
pub(super) fn decode_wav(bytes: &[u8]) -> Option<Pcm> {
    if bytes.get(..4)? != b"RIFF" || bytes.get(8..12)? != b"WAVE" {
        return None;
    }
    let (mut format, mut data) = (None, None);
    let mut at = 12_usize;
    while at + 8 <= bytes.len() {
        let size = u32::from_le_bytes(bytes[at + 4..at + 8].try_into().ok()?) as usize;
        let body = bytes.get(at + 8..(at + 8).checked_add(size)?.min(bytes.len()))?;
        match &bytes[at..at + 4] {
            b"fmt " if body.len() >= 16 => {
                let word = |i: usize| u16::from_le_bytes([body[i], body[i + 1]]);
                let rate = u32::from_le_bytes(body[4..8].try_into().ok()?);
                format = Some((word(0), word(2), rate, word(14)));
            }
            b"data" => data = Some(body),
            _ => {}
        }
        at += 8 + size + (size & 1);
    }
    let ((1, channels @ 1..=2, rate, 16), data) = (format?, data?) else {
        return None;
    };
    Some(Pcm {
        channels: channels as u8,
        rate,
        samples: data
            .chunks_exact(2)
            .map(|pair| i16::from_le_bytes([pair[0], pair[1]]))
            .collect::<Vec<_>>()
            .into(),
    })
}

fn load_file(view: &LayeredPackView, path: &str) -> Option<Pcm> {
    if let Some(bytes) = view.read_capped(&format!("{path}.fsb"), MAX_FILE_BYTES) {
        let sound = decode_fsb5(&bytes).ok()?;
        return Some(Pcm {
            channels: sound.channels,
            rate: sound.sample_rate,
            samples: sound.samples.into(),
        });
    }
    decode_wav(&view.read_capped(&format!("{path}.wav"), MAX_FILE_BYTES)?)
}

impl ServerSoundPack {
    /// `None` when the stack overrides no sound routing at all.
    pub(crate) fn from_view(view: &LayeredPackView) -> Option<Self> {
        let raw: Map<String, Value> = view.merged_sound_definitions();
        let definitions: HashMap<Box<str>, AudioDefinition> = raw
            .iter()
            .take(MAX_DEFINITIONS)
            .filter_map(|(name, value)| Some((Box::from(name.as_str()), definition(name, value)?)))
            .collect();
        let routing = view.merged_json_object("sounds.json", None);
        let tables = (!routing.is_empty())
            .then(|| SoundEventTables::from_json(&Value::Object(routing), &Value::Null))
            .filter(|tables| !tables.is_empty());
        if definitions.is_empty() && tables.is_none() {
            return None;
        }
        let mut files = HashMap::new();
        let mut budget = MAX_TOTAL_PCM_BYTES;
        for alt in definitions.values().flat_map(|def| def.alternatives.iter()) {
            if files.contains_key(&alt.name) {
                continue;
            }
            let Some(pcm) = load_file(view, &alt.name) else {
                continue;
            };
            let size = pcm.samples.len() * 2;
            if size > budget {
                break;
            }
            budget -= size;
            files.insert(alt.name.clone(), Arc::new(pcm));
        }
        Some(Self {
            definitions,
            tables,
            files,
        })
    }
}

static MAILBOX: Mutex<(u64, Option<Arc<ServerSoundPack>>)> = Mutex::new((0, None));

/// Publishes this session's server sounds (`None` view or no overrides clears them).
pub(crate) fn publish_server_sounds(view: Option<&LayeredPackView>) {
    let pack = view.and_then(ServerSoundPack::from_view).map(Arc::new);
    let mut mailbox = MAILBOX.lock().unwrap_or_else(|poison| poison.into_inner());
    mailbox.0 += 1;
    mailbox.1 = pack;
}

/// The newly published server sounds since `seen`, if any; `Some(None)` means cleared.
pub(crate) fn poll_server_sounds(seen: &mut u64) -> Option<Option<Arc<ServerSoundPack>>> {
    let mailbox = MAILBOX.lock().unwrap_or_else(|poison| poison.into_inner());
    if mailbox.0 == *seen {
        return None;
    }
    *seen = mailbox.0;
    Some(mailbox.1.clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn definition_parses_scalar_and_object_alternatives() {
        let parsed = definition(
            "custom.beep",
            &json!({
                "category": "ui", "min_distance": 2, "__use_legacy_max_distance": "true",
                "sounds": ["sounds/a", {"name": "sounds/b", "volume": 0.5, "weight": 3, "is3D": false}]
            }),
        )
        .expect("definition");
        assert_eq!(parsed.alternatives.len(), 2);
        assert!(!parsed.alternatives[0].object_form);
        assert_eq!(parsed.alternatives[1].weight, 3);
        assert_eq!(parsed.alternatives[1].is_3d, Some(false));
        assert_eq!(parsed.min_distance, Some(2.0));
        assert_eq!(parsed.use_legacy_max_distance.as_deref(), Some("true"));
        assert!(definition("x", &json!({"category": "ui"})).is_none());
    }

    fn wav(channels: u16, rate: u32, samples: &[i16]) -> Vec<u8> {
        let data: Vec<u8> = samples.iter().flat_map(|s| s.to_le_bytes()).collect();
        let mut out = b"RIFF".to_vec();
        out.extend((36 + data.len() as u32).to_le_bytes());
        out.extend(b"WAVEfmt ");
        out.extend(16_u32.to_le_bytes());
        for word in [1_u16, channels] {
            out.extend(word.to_le_bytes());
        }
        out.extend(rate.to_le_bytes());
        out.extend((rate * u32::from(channels) * 2).to_le_bytes());
        out.extend((channels * 2).to_le_bytes());
        out.extend(16_u16.to_le_bytes());
        out.extend(b"data");
        out.extend((data.len() as u32).to_le_bytes());
        out.extend(data);
        out
    }

    #[test]
    fn wav_pcm16_decodes_and_other_formats_are_skipped() {
        let pcm = decode_wav(&wav(2, 44_100, &[1, 2, 3, 4])).expect("wav");
        assert_eq!((pcm.channels, pcm.rate, pcm.frames()), (2, 44_100, 2));
        assert!(decode_wav(b"OggS").is_none());
        let mut float = wav(1, 8000, &[1]);
        float[20] = 3;
        assert!(decode_wav(&float).is_none());
    }

    #[test]
    fn mailbox_reports_each_publication_once() {
        publish_server_sounds(None);
        let mut seen = 0;
        assert!(matches!(poll_server_sounds(&mut seen), Some(None)));
        assert!(poll_server_sounds(&mut seen).is_none());
    }
}
