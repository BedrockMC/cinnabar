use std::{collections::HashMap, io::Cursor, sync::Arc};

use bevy::prelude::Resource;
use image::{ImageFormat, ImageReader, Limits};
use resource_pack::{LayeredPackView, PackAdmission, normalize_jsonc};
use serde_json::Value;

use super::{
    block_overlay::{CompiledBlockOverlay, compile_block_overlay},
    item_icons::compile_session_icons,
};
use crate::ui_runtime::presentation::SessionIcons;

/// Everything the session applies from its server pack stack.
#[derive(Debug)]
pub struct PackApplication {
    pub(crate) admission: PackAdmission,
    pub(crate) server_lang: Option<Arc<assets::ServerLangOverlay>>,
    pub(crate) block_overlay: Option<Arc<CompiledBlockOverlay>>,
    pub(crate) item_icons: Option<Arc<SessionIcons>>,
}

impl Default for PackApplication {
    fn default() -> Self {
        Self {
            admission: PackAdmission::None,
            server_lang: None,
            block_overlay: None,
            item_icons: None,
        }
    }
}

/// Reads StartGame's custom blocks and item icon keys, then applies the stack.
pub(super) fn prepare_session_packs(
    handoff: protocol::ResourcePackHandoff,
    game_data: &protocol::GameData,
) -> (protocol::CustomBlocks, PackApplication) {
    let custom_blocks = protocol::CustomBlocks::from_game_data(game_data);
    let icon_keys = protocol::item_icon_keys(game_data);
    let packs = prepare_pack_application(handoff, &custom_blocks, &icon_keys);
    (custom_blocks, packs)
}

pub(super) fn prepare_pack_application(
    handoff: protocol::ResourcePackHandoff,
    custom_blocks: &protocol::CustomBlocks,
    icon_keys: &[(Arc<str>, Arc<str>)],
) -> PackApplication {
    if handoff.is_empty() {
        return PackApplication::default();
    }
    let stack = resource_pack::validate_handoff(handoff);
    for rejection in stack.rejections() {
        bevy::log::warn!(
            stack_index = rejection.stack_index,
            reason = %rejection.reason,
            "server resource pack dropped"
        );
    }
    let view = LayeredPackView::new(Arc::clone(&stack));
    let block_overlay = compile_block_overlay(&view, custom_blocks).map(Arc::new);
    if let Some(compiled) = &block_overlay
        && compiled.gaps != Default::default()
    {
        bevy::log::warn!(gaps = ?compiled.gaps, "server block visuals are incomplete");
    }
    PackApplication {
        server_lang: merged_server_lang(&view),
        item_icons: compile_session_icons(&view, icon_keys),
        admission: PackAdmission::Validated(stack),
        block_overlay,
    }
}

/// Returns the carrier extended with this session's custom block visuals, or the
/// carrier itself when there is nothing to apply or the overlay does not fit.
pub(super) fn session_runtime_assets(
    base: &Arc<assets::RuntimeAssets>,
    custom_ids: Option<&std::ops::Range<u32>>,
    compiled: Option<&CompiledBlockOverlay>,
) -> Arc<assets::RuntimeAssets> {
    let (Some(ids), Some(compiled)) = (custom_ids, compiled) else {
        return Arc::clone(base);
    };
    if compiled.overlay.visuals.len() != ids.len() {
        bevy::log::warn!("server block visuals do not match the custom block ids");
        return Arc::clone(base);
    }
    match base.with_block_overlay(ids.start, &compiled.overlay) {
        Ok(assets) => Arc::new(assets),
        Err(error) => {
            bevy::log::warn!(%error, "server block visuals were not applied");
            Arc::clone(base)
        }
    }
}

/// Points the chunk renderer at the session's assets. Every switch takes a
/// fresh revision so the GPU tables re-upload even if an allocation is reused.
pub(super) fn install_chunk_textures(
    textures: &mut render::ChunkTextureAssets,
    assets: &Arc<assets::RuntimeAssets>,
) {
    static REVISION: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    if !Arc::ptr_eq(textures.assets(), assets) {
        let revision = REVISION.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        *textures = render::ChunkTextureAssets::with_revision(Arc::clone(assets), revision);
    }
}

const MAX_TEXTURE_SOURCE_BYTES: usize = 4 * 1024 * 1024;
const MAX_TEXTURE_SIDE: u32 = 1024;
const MAX_DECODE_ALLOC: u64 = 16 * 1024 * 1024;
pub(super) const MAX_CATALOG_ENTRIES: usize = 16_384;

/// Straight-alpha RGBA8 pixels decoded from a pack image.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct DecodedTexture {
    pub(super) width: u32,
    pub(super) height: u32,
    pub(super) rgba8: Box<[u8]>,
}

/// Maps each `texture_data` key of a texture catalog (terrain or item) to its
/// image path; a higher pack replaces a key.
pub(super) fn texture_key_paths(view: &LayeredPackView, catalog: &str) -> HashMap<String, String> {
    let mut paths = HashMap::new();
    for layer in view.read_layers(catalog) {
        let Some(Value::Object(data)) =
            parse_pack_json(&layer).map(|mut root| root["texture_data"].take())
        else {
            continue;
        };
        for (key, entry) in data {
            if paths.len() >= MAX_CATALOG_ENTRIES && !paths.contains_key(&key) {
                break;
            }
            if let Some(path) = first_texture_path(&entry["textures"]) {
                paths.insert(key, path);
            }
        }
    }
    paths
}

/// Decodes the winning image at `path`, trying `.png` then `.tga` as vanilla does.
pub(super) fn decode_pack_texture(view: &LayeredPackView, path: &str) -> Option<DecodedTexture> {
    [("png", ImageFormat::Png), ("tga", ImageFormat::Tga)]
        .into_iter()
        .find_map(|(extension, format)| {
            let bytes = view.read(&format!("{path}.{extension}"))?;
            decode_image(&bytes, format)
        })
}

pub(super) fn parse_pack_json(bytes: &[u8]) -> Option<Value> {
    serde_json::from_slice(&normalize_jsonc(bytes)?).ok()
}

/// A texture entry is a path, an object with `path`, or a variation list whose
/// first element is used.
fn first_texture_path(value: &Value) -> Option<String> {
    let path = match value {
        Value::String(path) => path.as_str(),
        Value::Object(entry) => entry.get("path")?.as_str()?,
        Value::Array(entries) => return first_texture_path(entries.first()?),
        _ => return None,
    };
    let path = path.trim().trim_start_matches("./");
    (!path.is_empty()).then(|| path.to_owned())
}

fn decode_image(bytes: &[u8], format: ImageFormat) -> Option<DecodedTexture> {
    if bytes.is_empty() || bytes.len() > MAX_TEXTURE_SOURCE_BYTES {
        return None;
    }
    let (width, height) = ImageReader::with_format(Cursor::new(bytes), format)
        .into_dimensions()
        .ok()?;
    if width == 0 || height == 0 || width > MAX_TEXTURE_SIDE || height > MAX_TEXTURE_SIDE {
        return None;
    }
    let mut reader = ImageReader::with_format(Cursor::new(bytes), format);
    let mut limits = Limits::default();
    limits.max_image_width = Some(MAX_TEXTURE_SIDE);
    limits.max_image_height = Some(MAX_TEXTURE_SIDE);
    limits.max_alloc = Some(MAX_DECODE_ALLOC);
    reader.limits(limits);
    let rgba8 = reader
        .decode()
        .ok()?
        .into_rgba8()
        .into_raw()
        .into_boxed_slice();
    Some(DecodedTexture {
        width,
        height,
        rgba8,
    })
}

/// The client requests `en_US` at login, so that is the only locale merged.
const SERVER_LANG_PATH: &str = "texts/en_US.lang";

/// Merges every pack's language file so a higher-precedence pack overrides a
/// key and keys it does not define still come from lower packs. Lowest layers
/// are dropped first if the merged text would exceed the overlay input bound.
fn merged_server_lang(view: &LayeredPackView) -> Option<Arc<assets::ServerLangOverlay>> {
    let mut kept = Vec::new();
    let mut total = 0usize;
    for layer in view.read_layers(SERVER_LANG_PATH).into_iter().rev() {
        let text = layer
            .strip_prefix(b"\xef\xbb\xbf")
            .unwrap_or(&layer)
            .to_vec();
        let Some(next) = total.checked_add(text.len() + 1) else {
            break;
        };
        if next > assets::MAX_SERVER_LANG_INPUT_BYTES {
            break;
        }
        total = next;
        kept.push(text);
    }
    if kept.is_empty() {
        return None;
    }
    // The overlay keeps the last definition of a key, so write lowest first.
    let mut merged = Vec::with_capacity(total);
    for text in kept.iter().rev() {
        merged.extend_from_slice(text);
        merged.push(b'\n');
    }
    assets::ServerLangOverlay::read(merged.len(), |output| {
        output.copy_from_slice(&merged);
        true
    })
}

pub(super) fn install_session_icons(
    runtime: &mut crate::ui_runtime::UiRuntime,
    generation: u64,
    icons: Option<Arc<SessionIcons>>,
    setup_succeeded: bool,
) {
    if runtime.session_id() == generation {
        runtime.set_session_icons(icons.filter(|_| setup_succeeded));
    }
}

pub(super) fn install_server_language(
    runtime: &mut crate::ui_runtime::UiRuntime,
    generation: u64,
    overlay: Option<std::sync::Arc<assets::ServerLangOverlay>>,
    setup_succeeded: bool,
) {
    if runtime.session_id() == generation {
        runtime.set_server_lang(if setup_succeeded { overlay } else { None });
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum BootstrapGenerationDisposition {
    Expected,
    Stale,
    Unexpected,
}

pub(crate) const fn classify_bootstrap_generation(
    ui_generation: u64,
    world_generation: u64,
    incoming_generation: u64,
) -> BootstrapGenerationDisposition {
    let directly_next = ui_generation == world_generation
        && matches!(
            world_generation.checked_add(1),
            Some(expected) if expected == incoming_generation
        );
    let pending_ui_generation =
        incoming_generation == ui_generation && incoming_generation > world_generation;
    if directly_next || pending_ui_generation {
        BootstrapGenerationDisposition::Expected
    } else if incoming_generation <= world_generation || incoming_generation < ui_generation {
        BootstrapGenerationDisposition::Stale
    } else {
        BootstrapGenerationDisposition::Unexpected
    }
}

/// Generation-bound admission for the current session's optional pack stack.
/// This owns validated bytes independently of optional language application.
#[derive(Debug, Resource)]
pub(crate) struct ResourcePackAdmissionState {
    generation: u64,
    admission: PackAdmission,
}

impl Default for ResourcePackAdmissionState {
    fn default() -> Self {
        Self {
            generation: 0,
            admission: PackAdmission::None,
        }
    }
}

impl ResourcePackAdmissionState {
    /// Starts ownership for a pending generation and releases the prior stack.
    pub(crate) fn begin_generation(&mut self, generation: u64) -> bool {
        if generation <= self.generation {
            return false;
        }
        self.generation = generation;
        self.admission = PackAdmission::None;
        true
    }

    /// Publishes admission only for the pending/current or a newer generation.
    pub(crate) fn replace_for_generation(
        &mut self,
        generation: u64,
        admission: PackAdmission,
    ) -> bool {
        if generation < self.generation {
            return false;
        }
        self.generation = generation;
        self.admission = admission;
        true
    }

    /// Releases admission when the current network session terminates.
    pub(crate) fn clear_current(&mut self) {
        self.admission = PackAdmission::None;
    }

    #[cfg(test)]
    pub(crate) const fn generation(&self) -> u64 {
        self.generation
    }

    #[cfg(test)]
    pub(crate) const fn admission(&self) -> &PackAdmission {
        &self.admission
    }
}

#[cfg(test)]
mod tests {
    use resource_pack::{AdmissionError, PackAdmission};

    use super::ResourcePackAdmissionState;

    #[test]
    fn absent_or_rejected_application_preserves_optional_admission() {
        let application = super::prepare_pack_application(
            protocol::ResourcePackHandoff::default(),
            &protocol::CustomBlocks::default(),
            &[],
        );
        assert!(matches!(application.admission, PackAdmission::None));
        assert!(application.server_lang.is_none());
        let pack = protocol::ResourcePackArchive::unencrypted(
            "11111111-2222-3333-4444-555555555555".parse().unwrap(),
            "1.2.3".into(),
            String::new(),
            vec![0; 32],
        );
        let application = super::prepare_pack_application(
            protocol::ResourcePackHandoff::from_archives(vec![pack]),
            &protocol::CustomBlocks::default(),
            &[],
        );
        let overlay = application.server_lang;
        let PackAdmission::Validated(stack) = application.admission else {
            panic!("a dropped pack still yields an admitted stack");
        };
        assert!(stack.packs().is_empty());
        assert_eq!(
            stack.rejections()[0].reason,
            AdmissionError::InvalidZipFooter
        );
        assert!(overlay.is_none());
    }

    fn lang_pack(id: u128, lang: &[u8]) -> protocol::ResourcePackArchive {
        use std::io::Write;
        let id = format!("00000000-0000-0000-0000-{id:012x}");
        let manifest = format!(
            r#"{{"format_version":2,"header":{{"uuid":"{id}","version":[1,0,0]}},"modules":[{{"type":"resources"}}]}}"#
        );
        let mut writer = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        for (path, bytes) in [
            ("manifest.json", manifest.as_bytes()),
            ("texts/en_US.lang", lang),
        ] {
            writer
                .start_file(path, zip::write::SimpleFileOptions::default())
                .unwrap();
            writer.write_all(bytes).unwrap();
        }
        let archive = writer.finish().unwrap().into_inner();
        protocol::ResourcePackArchive::unencrypted(
            id.parse().unwrap(),
            "1.0.0".into(),
            String::new(),
            archive,
        )
    }

    // Higher packs override shared keys; keys only a lower pack defines survive.
    #[test]
    fn language_files_merge_across_the_stack_by_precedence() {
        let handoff = protocol::ResourcePackHandoff::from_archives(vec![
            lang_pack(1, b"shared=top\ntop.only=T"),
            lang_pack(2, b"\xef\xbb\xbfshared=bottom\nbottom.only=B"),
        ]);
        let application =
            super::prepare_pack_application(handoff, &protocol::CustomBlocks::default(), &[]);
        let overlay = application.server_lang.expect("merged overlay");
        assert_eq!(overlay.lookup("shared"), Some("top"));
        assert_eq!(overlay.lookup("top.only"), Some("T"));
        assert_eq!(overlay.lookup("bottom.only"), Some("B"));
    }

    #[test]
    fn newer_generation_replaces_atomically_and_stale_results_are_ignored() {
        let mut state = ResourcePackAdmissionState::default();
        assert!(state.begin_generation(2));
        assert!(matches!(state.admission(), PackAdmission::None));
        let stack = resource_pack::validate_handoff(protocol::ResourcePackHandoff::default());
        assert!(state.replace_for_generation(2, PackAdmission::Validated(stack)));
        assert!(!state.replace_for_generation(1, PackAdmission::None));
        assert_eq!(state.generation(), 2);
        assert!(matches!(state.admission(), PackAdmission::Validated(_)));
        assert!(state.begin_generation(3));
        assert!(matches!(state.admission(), PackAdmission::None));
        assert!(!state.begin_generation(2));
        state.clear_current();
        assert_eq!(state.generation(), 3);
        assert!(matches!(state.admission(), PackAdmission::None));
    }
}
