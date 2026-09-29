//! Startup-loaded sound bank: routing tables, definition catalog, on-demand FSB decode and cache.

use std::{
    collections::{HashMap, HashSet, VecDeque},
    fs::File,
    io::{Read, Seek, SeekFrom},
    path::{Path, PathBuf},
    sync::Arc,
};

use assets::{
    AudioDefinition, RuntimeAudioCatalog, SoundBankEntry, SoundBankIndex, SoundEventTables,
    decode_sound, sound_bank_prefix_len,
};
use serde_json::Value;

use super::{server::ServerSoundPack, voice::Pcm};

pub(crate) const SOUND_BANK_FILENAME: &str = "vanilla-v1.mcbesnd";
const HEADER_READ_BYTES: usize = 40;
const CACHE_BUDGET_BYTES: usize = 96 * 1024 * 1024;

/// Where the sound bank sits relative to the world carrier.
pub(crate) fn sound_bank_path(world_asset_path: &Path) -> PathBuf {
    world_asset_path.with_file_name(SOUND_BANK_FILENAME)
}

/// One `music_definitions.json` record: the sound event and the gap before the next track.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct MusicEntry {
    pub event_name: Box<str>,
    pub min_delay: f32,
    pub max_delay: f32,
}

pub(crate) struct SoundBank {
    file: Option<File>,
    index: SoundBankIndex,
    tables: SoundEventTables,
    catalog: Option<Arc<RuntimeAudioCatalog>>,
    server: Option<Arc<ServerSoundPack>>,
    merged: Option<SoundEventTables>,
    music: HashMap<Box<str>, MusicEntry>,
    cache: HashMap<Box<str>, Arc<Pcm>>,
    cache_order: VecDeque<Box<str>>,
    cache_bytes: usize,
    failed: HashSet<Box<str>>,
}

fn parse_music(bytes: &[u8]) -> HashMap<Box<str>, MusicEntry> {
    let Ok(Value::Object(map)) = serde_json::from_slice::<Value>(bytes) else {
        return HashMap::new();
    };
    map.iter()
        .filter_map(|(key, value)| {
            let event_name = value.get("event_name")?.as_str()?;
            let delay = |name: &str, fallback: f64| {
                value.get(name).and_then(Value::as_f64).unwrap_or(fallback) as f32
            };
            Some((
                Box::from(key.as_str()),
                MusicEntry {
                    event_name: event_name.into(),
                    min_delay: delay("min_delay", 60.0),
                    max_delay: delay("max_delay", 180.0),
                },
            ))
        })
        .collect()
}

impl SoundBank {
    /// Opens the bank file; `Ok(None)` when absent so audio stays silent instead of failing startup.
    pub(crate) fn open(
        path: &Path,
        catalog: Option<Arc<RuntimeAudioCatalog>>,
    ) -> Result<Option<Self>, String> {
        let mut file = match File::open(path) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(format!("open {}: {error}", path.display())),
        };
        let mut header = [0_u8; HEADER_READ_BYTES];
        file.read_exact(&mut header)
            .map_err(|error| format!("read {}: {error}", path.display()))?;
        let prefix_len = sound_bank_prefix_len(&header).map_err(|error| error.to_string())?;
        let mut prefix = vec![0_u8; prefix_len];
        prefix[..HEADER_READ_BYTES].copy_from_slice(&header);
        file.read_exact(&mut prefix[HEADER_READ_BYTES..])
            .map_err(|error| format!("read {}: {error}", path.display()))?;
        let index = SoundBankIndex::decode_prefix(&prefix).map_err(|error| error.to_string())?;
        let json = |bytes: &[u8]| serde_json::from_slice::<Value>(bytes).unwrap_or(Value::Null);
        let tables =
            SoundEventTables::from_json(&json(index.sounds_json()), &json(index.materials_json()));
        Ok(Some(Self {
            file: Some(file),
            music: parse_music(index.music_json()),
            index,
            tables,
            catalog,
            server: None,
            merged: None,
            cache: HashMap::new(),
            cache_order: VecDeque::new(),
            cache_bytes: 0,
            failed: HashSet::new(),
        }))
    }

    /// Vanilla routing with the server pack's `sounds.json` layered on top.
    pub(crate) fn tables(&self) -> &SoundEventTables {
        self.merged.as_ref().unwrap_or(&self.tables)
    }

    pub(crate) fn music(&self, key: &str) -> Option<&MusicEntry> {
        self.music.get(key)
    }

    pub(crate) fn file_count(&self) -> usize {
        self.index.len()
    }

    /// Replaces the session's server pack; `None` restores vanilla definitions.
    pub(crate) fn install_server(&mut self, pack: Option<Arc<ServerSoundPack>>) {
        self.merged = pack
            .as_ref()
            .and_then(|pack| pack.tables.clone())
            .map(|overlay| {
                let mut merged = self.tables.clone();
                merged.merge(overlay);
                merged
            });
        self.server = pack;
    }

    /// Definition by name, the server pack winning over the vanilla catalog.
    pub(crate) fn definition(&self, name: &str) -> Option<&AudioDefinition> {
        self.server
            .as_ref()
            .and_then(|pack| pack.definitions.get(name))
            .or_else(|| self.catalog.as_ref()?.lookup(name))
    }

    /// Decoded PCM for an alternative's sound path (no extension); non-streaming sounds are cached.
    pub(crate) fn pcm(&mut self, path: &str, stream: bool) -> Option<Arc<Pcm>> {
        if let Some(found) = self.server.as_ref().and_then(|pack| pack.files.get(path)) {
            return Some(Arc::clone(found));
        }
        if let Some(found) = self.cache.get(path) {
            return Some(Arc::clone(found));
        }
        if self.failed.contains(path) {
            return None;
        }
        let entry = self.index.entry(path);
        let decoded = entry
            .and_then(|entry| self.read_entry(entry))
            .and_then(|bytes| {
                decode_sound(&bytes)
                    .map_err(|error| bevy::log::debug!(%error, path, "sound decode failed"))
                    .ok()
            });
        let Some(sound) = decoded else {
            self.failed.insert(path.into());
            return None;
        };
        let pcm = Arc::new(Pcm {
            channels: sound.channels,
            rate: sound.sample_rate,
            samples: sound.samples.into(),
        });
        if !stream {
            self.remember(path, &pcm);
        }
        Some(pcm)
    }

    fn read_entry(&mut self, entry: SoundBankEntry) -> Option<Vec<u8>> {
        let file = self.file.as_mut()?;
        let mut bytes = vec![0_u8; entry.len as usize];
        file.seek(SeekFrom::Start(entry.offset)).ok()?;
        file.read_exact(&mut bytes).ok()?;
        Some(bytes)
    }

    fn remember(&mut self, path: &str, pcm: &Arc<Pcm>) {
        let size = pcm.samples.len() * 2;
        while self.cache_bytes + size > CACHE_BUDGET_BYTES {
            let Some(oldest) = self.cache_order.pop_front() else {
                break;
            };
            if let Some(evicted) = self.cache.remove(&oldest) {
                self.cache_bytes -= evicted.samples.len() * 2;
            }
        }
        self.cache.insert(path.into(), Arc::clone(pcm));
        self.cache_order.push_back(path.into());
        self.cache_bytes += size;
    }

    #[cfg(test)]
    pub(crate) fn insert_test_pcm(&mut self, path: &str, pcm: Arc<Pcm>) {
        self.remember(path, &pcm);
    }

    #[cfg(test)]
    pub(crate) fn for_test(
        index: SoundBankIndex,
        tables: SoundEventTables,
        catalog: Option<Arc<RuntimeAudioCatalog>>,
    ) -> Self {
        Self {
            file: None,
            music: parse_music(index.music_json()),
            index,
            tables,
            catalog,
            server: None,
            merged: None,
            cache: HashMap::new(),
            cache_order: VecDeque::new(),
            cache_bytes: 0,
            failed: HashSet::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn absent_bank_is_none_and_music_parses() {
        let missing = std::env::temp_dir().join("cinnabar-no-such-sound-bank.mcbesnd");
        assert!(SoundBank::open(&missing, None).unwrap().is_none());
        let music = parse_music(
            br#"{"creative":{"event_name":"music.game.creative","min_delay":30,"max_delay":90}}"#,
        );
        assert_eq!(music["creative"].max_delay, 90.0);
    }

    #[test]
    fn open_reads_an_encoded_bank_and_decodes_a_pcm_file() {
        // One PCM16 mono FSB5 of two frames at 48 kHz.
        let mut fsb = b"FSB5".to_vec();
        let mode = (9_u64 << 1) | (2_u64 << 34);
        for value in [1_u32, 1, 8, 0, 4, 2, 0, 0] {
            fsb.extend(value.to_le_bytes());
        }
        fsb.resize(60, 0);
        fsb.extend(mode.to_le_bytes());
        fsb.extend([0, 0x40, 0, 0xc0]);
        let bytes = assets::encode_sound_bank(
            br#"{}"#,
            b"{}",
            b"{}",
            &[("sounds/test/tone".to_owned(), fsb)],
        )
        .expect("encode");
        let path =
            std::env::temp_dir().join(format!("cinnabar-bank-{}.mcbesnd", std::process::id()));
        std::fs::write(&path, bytes).expect("write");
        let mut bank = SoundBank::open(&path, None)
            .expect("open")
            .expect("present");
        let pcm = bank.pcm("sounds/test/tone", false).expect("decode");
        assert_eq!((pcm.channels, pcm.rate, pcm.frames()), (1, 48_000, 2));
        assert!(bank.pcm("sounds/test/missing", false).is_none());
        assert_eq!(bank.file_count(), 1);
        let _ = std::fs::remove_file(&path);
    }
}
