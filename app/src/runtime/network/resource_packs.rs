use std::{collections::HashMap, io::Cursor, sync::Arc};

use bevy::prelude::Resource;
use image::{ImageFormat, ImageReader, Limits};
use resource_pack::{LayeredPackView, PackAdmission, normalize_jsonc};
use serde_json::Value;

use super::{
    block_overlay::{CompiledBlockOverlay, compile_block_overlay},
    glyph_sheets::compile_session_glyphs,
    item_icons::{BlockIcons, compile_session_icons, custom_block_icons, custom_block_items},
};
use crate::ui_runtime::presentation::{ServerUiPack, SessionGlyphSheets, SessionIcons};

/// Everything the session applies from its server pack stack.
#[derive(Debug)]
pub struct PackApplication {
    /// Inert metadata only; accepting a pack does not authorize execution.
    pub(crate) extension_marker: Option<Arc<[u8]>>,
    pub(crate) admission: PackAdmission,
    pub(crate) server_lang: Option<Arc<assets::ServerLangOverlay>>,
    pub(crate) block_overlay: Option<Arc<CompiledBlockOverlay>>,
    pub(crate) item_icons: Option<Arc<SessionIcons>>,
    /// StartGame item components, applied with the pack stack they present through.
    pub(crate) item_components: Option<Arc<crate::ui_runtime::item_facts::SessionItemComponents>>,
    pub(crate) glyph_sheets: Option<Arc<SessionGlyphSheets>>,
    pub(crate) entities: Option<Arc<super::entity_pack::SessionEntityPack>>,
    pub(crate) property_defaults: Vec<(Arc<str>, Vec<client_world::PropertyDefault>)>,
    pub(crate) server_ui: Option<Arc<ServerUiPack>>,
    /// Installed only once the session's Bootstrap is accepted.
    pub(crate) server_sounds: Option<Arc<crate::audio::ServerSoundPack>>,
}

impl Default for PackApplication {
    fn default() -> Self {
        Self {
            extension_marker: None,
            admission: PackAdmission::None,
            server_lang: None,
            block_overlay: None,
            item_icons: None,
            item_components: None,
            glyph_sheets: None,
            entities: None,
            property_defaults: Vec::new(),
            server_ui: None,
            server_sounds: None,
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
    let block_items = custom_block_items(game_data, &custom_blocks);
    let hashed = game_data.start_game.block_network_ids_are_hashes;
    let mut packs =
        prepare_pack_application(handoff, &custom_blocks, &icon_keys, &block_items, hashed);
    packs.item_components =
        crate::ui_runtime::item_facts::SessionItemComponents::from_game_data(game_data);
    (custom_blocks, packs)
}

/// `block_items` pairs each custom block item with the block it draws as.
pub(super) fn prepare_pack_application(
    handoff: protocol::ResourcePackHandoff,
    custom_blocks: &protocol::CustomBlocks,
    icon_keys: &[(Arc<str>, Arc<str>)],
    block_items: &[(Arc<str>, Arc<str>)],
    hashed_block_ids: bool,
) -> PackApplication {
    if handoff.is_empty() {
        super::item_diagnostics::session_icons(icon_keys.len(), None);
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
    let fingerprint = stack_fingerprint(&stack);
    let block_overlay = cached_block_overlay(&fingerprint, custom_blocks, hashed_block_ids, || {
        compile_block_overlay(
            &view,
            custom_blocks,
            hashed_block_ids,
            BASE_MATERIAL_KEYS.get(),
        )
        .map(Arc::new)
    });
    if let Some(compiled) = &block_overlay
        && compiled.gaps != Default::default()
    {
        bevy::log::warn!(gaps = ?compiled.gaps, "server block visuals are incomplete");
    }
    let block_icons = block_overlay
        .as_deref()
        .map_or_else(BlockIcons::default, |compiled| {
            custom_block_icons(
                &compiled.overlay,
                custom_blocks,
                hashed_block_ids,
                block_items,
            )
        });
    let item_icons = compile_session_icons(&view, icon_keys, block_icons);
    super::item_diagnostics::session_icons(icon_keys.len(), item_icons.as_deref());
    PackApplication {
        extension_marker: view
            .read_capped(
                server_experience::policy::MARKER_PATH,
                server_experience::policy::MAX_MARKER_BYTES as u64,
            )
            .map(Arc::from),
        server_lang: merged_server_lang(&view),
        item_icons,
        item_components: None,
        glyph_sheets: compile_session_glyphs(&view),
        entities: super::entity_pack::compile_session_entities(&fingerprint, &view),
        property_defaults: super::entity_pack::pack_property_defaults(&view),
        server_ui: collect_server_ui(&view),
        server_sounds: crate::audio::ServerSoundPack::from_view(&view).map(Arc::new),
        admission: PackAdmission::Validated(stack),
        block_overlay,
    }
}

/// Bounds on the pack UI handed to the form engine.
const MAX_SERVER_UI_BYTES: usize = 16 * 1024 * 1024;
const MAX_SERVER_UI_TEXTURES: usize = 4096;

/// Each pack's `ui/**/*.json` (lowest precedence first) and the winning texture
/// files that ui references; `None` when no pack carries ui.
fn collect_server_ui(view: &LayeredPackView) -> Option<Arc<ServerUiPack>> {
    let mut total = 0usize;
    let mut pack = ServerUiPack::default();
    for layer in view.layers() {
        let mut files = Vec::new();
        for path in layer
            .files_under("ui/")
            .iter()
            .filter(|path| path.ends_with(".json"))
        {
            let Some(bytes) = layer.read_file(path).ok().flatten() else {
                continue;
            };
            total = total.saturating_add(bytes.len());
            if total > MAX_SERVER_UI_BYTES {
                break;
            }
            files.push(((*path).to_owned(), bytes.into_vec()));
        }
        pack.ui_layers.push(files);
    }
    if pack.is_empty() {
        return None;
    }
    let dirs = ServerUiPack::referenced_texture_dirs(&pack.ui_layers);
    for path in view.list("textures/") {
        if pack.textures.len() >= MAX_SERVER_UI_TEXTURES || total > MAX_SERVER_UI_BYTES {
            break;
        }
        if !ServerUiPack::wants_texture(&dirs, path) {
            continue;
        }
        if let Some(bytes) = view.read_capped(path, MAX_TEXTURE_SOURCE_BYTES as u64) {
            total = total.saturating_add(bytes.len());
            pack.textures.push((path.to_owned(), bytes.into_vec()));
        }
    }
    Some(Arc::new(pack))
}

pub(super) fn install_server_ui(
    runtime: &mut crate::ui_runtime::UiRuntime,
    generation: u64,
    pack: Option<Arc<ServerUiPack>>,
    setup_succeeded: bool,
) {
    if runtime.session_id() == generation {
        runtime.set_server_ui(pack.filter(|_| setup_succeeded));
    }
}

/// Texture keys of the vanilla carrier's materials, set once at startup when the sidecar loads.
static BASE_MATERIAL_KEYS: std::sync::OnceLock<assets::MaterialKeys> = std::sync::OnceLock::new();

pub(crate) fn set_base_material_keys(keys: assets::MaterialKeys) {
    let _ = BASE_MATERIAL_KEYS.set(keys);
}

/// The UI language's `texts/<code>.lang`, set once at startup; unset means en_US only.
static ACTIVE_LANG_PATH: std::sync::OnceLock<String> = std::sync::OnceLock::new();

pub(crate) fn set_active_language(code: &str) {
    if code != "en_US" {
        let _ = ACTIVE_LANG_PATH.set(format!("texts/{code}.lang"));
    }
}

pub(super) type StackFingerprint = Vec<(String, String, String, [u8; 32])>;

struct CachedOverlay {
    stack: StackFingerprint,
    hashed: bool,
    blocks: protocol::CustomBlocks,
    overlay: Option<Arc<CompiledBlockOverlay>>,
}

/// The previous session's compiled overlay, reused when the same pack stack and
/// block definitions rejoin.
static OVERLAY_CACHE: std::sync::Mutex<Option<CachedOverlay>> = std::sync::Mutex::new(None);

/// Identity of each pack as (uuid, version, subpack, content hash), in stack order.
pub(super) fn stack_fingerprint(stack: &resource_pack::ValidatedPackStack) -> StackFingerprint {
    use sha2::{Digest, Sha256};
    stack
        .packs()
        .iter()
        .map(|pack| {
            (
                pack.pack_id().to_string(),
                pack.version().to_owned(),
                pack.sub_pack_name().to_owned(),
                Sha256::digest(&*pack.archive_bytes()).into(),
            )
        })
        .collect()
}

/// Reuses compiled blocks with the fingerprint already computed for this admission.
fn cached_block_overlay(
    fingerprint: &StackFingerprint,
    blocks: &protocol::CustomBlocks,
    hashed: bool,
    compile: impl FnOnce() -> Option<Arc<CompiledBlockOverlay>>,
) -> Option<Arc<CompiledBlockOverlay>> {
    let mut cache = OVERLAY_CACHE
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    if let Some(cached) = cache.as_ref()
        && cached.hashed == hashed
        && cached.stack == *fingerprint
        && cached.blocks == *blocks
    {
        return cached.overlay.clone();
    }
    let overlay = compile();
    *cache = Some(CachedOverlay {
        stack: fingerprint.clone(),
        hashed,
        blocks: blocks.clone(),
        overlay: overlay.clone(),
    });
    overlay
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

const IMAGE_EXTENSIONS: [(&str, ImageFormat); 4] = [
    ("png", ImageFormat::Png),
    ("tga", ImageFormat::Tga),
    ("jpg", ImageFormat::Jpeg),
    ("jpeg", ImageFormat::Jpeg),
];
const MAX_TEXTURE_SET_BYTES: u64 = 64 * 1024;

/// Decodes the winning image at `path`, trying `.png`, `.tga`, and `.jpg` as
/// vanilla does, then the color layer of a `.texture_set.json`.
pub(super) fn decode_pack_texture(view: &LayeredPackView, path: &str) -> Option<DecodedTexture> {
    decode_image_file(view, path).or_else(|| decode_texture_set(view, path))
}

fn decode_image_file(view: &LayeredPackView, path: &str) -> Option<DecodedTexture> {
    // Vanilla's loader also tries the literal path, so a pack path that already
    // names its image (`textures/items/gem.png`) resolves.
    let named = path.rsplit_once('.').and_then(|(_, extension)| {
        IMAGE_EXTENSIONS
            .into_iter()
            .find(|(known, _)| extension.eq_ignore_ascii_case(known))
    });
    if let Some((_, format)) = named
        && let Some(texture) = view
            .read_capped(path, MAX_TEXTURE_SOURCE_BYTES as u64)
            .and_then(|bytes| decode_image(&bytes, format))
    {
        return Some(texture);
    }
    IMAGE_EXTENSIONS
        .into_iter()
        .find_map(|(extension, format)| {
            let bytes = view.read_capped(
                &format!("{path}.{extension}"),
                MAX_TEXTURE_SOURCE_BYTES as u64,
            )?;
            decode_image(&bytes, format)
        })
}

/// A texture set's `color` is a sibling image name or a solid `[r, g, b(, a)]`.
fn decode_texture_set(view: &LayeredPackView, path: &str) -> Option<DecodedTexture> {
    let bytes = view.read_capped(&format!("{path}.texture_set.json"), MAX_TEXTURE_SET_BYTES)?;
    let root = parse_pack_json(&bytes)?;
    match root.get("minecraft:texture_set")?.get("color")? {
        Value::String(name) => {
            let name = name.trim().trim_start_matches("./");
            let sibling = path
                .rsplit_once('/')
                .map_or_else(|| name.to_owned(), |(dir, _)| format!("{dir}/{name}"));
            [sibling, name.to_owned()]
                .into_iter()
                .filter(|target| target != path)
                .find_map(|target| decode_image_file(view, &target))
        }
        Value::Array(channels) if matches!(channels.len(), 3 | 4) => {
            let mut pixel = [0, 0, 0, 255];
            for (slot, channel) in pixel.iter_mut().zip(channels) {
                *slot = channel.as_f64()?.round().clamp(0.0, 255.0) as u8;
            }
            Some(DecodedTexture {
                width: 1,
                height: 1,
                rgba8: pixel.into(),
            })
        }
        _ => None,
    }
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

const SERVER_LANG_PATH: &str = "texts/en_US.lang";

/// Merges every pack's language file so a higher-precedence pack overrides a
/// key and keys it does not define still come from lower packs; the UI
/// language's files override en_US. Lowest layers are dropped first if the
/// merged text would exceed the overlay input bound.
fn merged_server_lang(view: &LayeredPackView) -> Option<Arc<assets::ServerLangOverlay>> {
    let mut kept = Vec::new();
    let mut total = 0usize;
    let mut layers = view.read_layers(SERVER_LANG_PATH);
    if let Some(active) = ACTIVE_LANG_PATH.get() {
        layers.extend(view.read_layers(active));
    }
    for layer in layers.into_iter().rev() {
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
    items: Option<Arc<crate::ui_runtime::item_facts::SessionItemComponents>>,
    setup_succeeded: bool,
) {
    if runtime.session_id() == generation {
        runtime.set_session_icons(icons.filter(|_| setup_succeeded));
        runtime.set_session_items(items.filter(|_| setup_succeeded));
    }
}

pub(super) fn install_session_glyphs(
    runtime: &mut crate::ui_runtime::UiRuntime,
    generation: u64,
    glyphs: Option<Arc<SessionGlyphSheets>>,
    setup_succeeded: bool,
) {
    if runtime.session_id() == generation {
        runtime.set_session_glyphs(glyphs.filter(|_| setup_succeeded));
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

    /// Serializes tests that go through the process-wide block overlay cache.
    static OVERLAY_CACHE_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn overlay_cache() -> std::sync::MutexGuard<'static, ()> {
        OVERLAY_CACHE_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    #[test]
    fn absent_or_rejected_application_preserves_optional_admission() {
        let _cache = overlay_cache();
        let application = super::prepare_pack_application(
            protocol::ResourcePackHandoff::default(),
            &protocol::CustomBlocks::default(),
            &[],
            &[],
            false,
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
            &[],
            false,
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
        archive(id, &[("texts/en_US.lang", lang)])
    }

    fn archive(id: u128, files: &[(&str, &[u8])]) -> protocol::ResourcePackArchive {
        use std::io::Write;
        let id = format!("00000000-0000-0000-0000-{id:012x}");
        let manifest = format!(
            r#"{{"format_version":2,"header":{{"uuid":"{id}","version":[1,0,0]}},"modules":[{{"type":"resources"}}]}}"#
        );
        let mut writer = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        for (path, bytes) in
            std::iter::once(("manifest.json", manifest.as_bytes())).chain(files.iter().copied())
        {
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

    fn files_pack(files: &[(&str, &[u8])]) -> resource_pack::LayeredPackView {
        use std::io::Write;
        let id = "00000000-0000-0000-0000-00000000000a";
        let manifest = format!(
            r#"{{"format_version":2,"header":{{"uuid":"{id}","version":[1,0,0]}},"modules":[{{"type":"resources"}}]}}"#
        );
        let mut writer = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        for (path, bytes) in
            std::iter::once(("manifest.json", manifest.as_bytes())).chain(files.iter().copied())
        {
            writer
                .start_file(path, zip::write::SimpleFileOptions::default())
                .unwrap();
            writer.write_all(bytes).unwrap();
        }
        let archive = protocol::ResourcePackArchive::unencrypted(
            id.parse().unwrap(),
            "1.0.0".into(),
            String::new(),
            writer.finish().unwrap().into_inner(),
        );
        resource_pack::LayeredPackView::new(resource_pack::validate_handoff(
            protocol::ResourcePackHandoff::from_archives(vec![archive]),
        ))
    }

    // A cancelled session's late preparation must not replace the live session's sounds.
    #[test]
    fn preparing_packs_leaves_sound_publication_to_bootstrap() {
        let _cache = overlay_cache();
        let _mailbox = crate::audio::SERVER_SOUNDS_TEST_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let before = crate::audio::server_sounds_generation();
        let sounds = br#"{"sound_definitions":{"custom.beep":{"sounds":["sounds/beep"]}}}"#;
        let application = super::prepare_pack_application(
            protocol::ResourcePackHandoff::from_archives(vec![archive(
                3,
                &[("sounds/sound_definitions.json", sounds)],
            )]),
            &protocol::CustomBlocks::default(),
            &[],
            &[],
            false,
        );
        assert_eq!(crate::audio::server_sounds_generation(), before);
        assert!(application.server_sounds.is_some(), "carried to Bootstrap");
    }

    // A texture set resolves to its sibling color image or a solid color.
    #[test]
    fn texture_sets_supply_color_layers() {
        let mut png = std::io::Cursor::new(Vec::new());
        image::RgbaImage::from_pixel(2, 2, image::Rgba([1, 2, 3, 255]))
            .write_to(&mut png, image::ImageFormat::Png)
            .unwrap();
        let png = png.into_inner();
        let view = files_pack(&[
            (
                "textures/blocks/a.texture_set.json",
                br#"{"format_version":"1.16.100","minecraft:texture_set":{"color":"a_color"}}"#,
            ),
            ("textures/blocks/a_color.png", &png),
            (
                "textures/blocks/b.texture_set.json",
                br#"{"minecraft:texture_set":{"color":[10,20,30]}}"#,
            ),
        ]);
        let image = super::decode_pack_texture(&view, "textures/blocks/a").expect("sibling");
        assert_eq!(
            (image.width, image.height, image.rgba8[..4].to_vec()),
            (2, 2, vec![1, 2, 3, 255])
        );
        let solid = super::decode_pack_texture(&view, "textures/blocks/b").expect("solid");
        assert_eq!(solid.rgba8.to_vec(), vec![10, 20, 30, 255]);
    }

    // Higher packs override shared keys; keys only a lower pack defines survive.
    #[test]
    fn language_files_merge_across_the_stack_by_precedence() {
        let _cache = overlay_cache();
        let handoff = protocol::ResourcePackHandoff::from_archives(vec![
            lang_pack(2, b"\xef\xbb\xbfshared=bottom\nbottom.only=B"),
            lang_pack(1, b"shared=top\ntop.only=T"),
        ]);
        let application = super::prepare_pack_application(
            handoff,
            &protocol::CustomBlocks::default(),
            &[],
            &[],
            false,
        );
        let overlay = application.server_lang.expect("merged overlay");
        assert_eq!(overlay.lookup("shared"), Some("top"));
        assert_eq!(overlay.lookup("top.only"), Some("T"));
        assert_eq!(overlay.lookup("bottom.only"), Some("B"));
    }

    // The same stack and blocks reuse the compiled overlay instead of recompiling.
    #[test]
    fn overlay_cache_reuses_the_previous_session_compile() {
        let _cache = overlay_cache();
        let blocks = protocol::CustomBlocks {
            blocks: vec![protocol::CustomBlock {
                name: "cache:test".into(),
                state_count: 1,
                collides: true,
                collision_box: None,
                selection: Default::default(),
                visual: Default::default(),
            }]
            .into(),
            skipped: 0,
        };
        let stack =
            resource_pack::validate_handoff(protocol::ResourcePackHandoff::from_archives(vec![
                lang_pack(7, b"a=b"),
            ]));
        let mut compiles = 0;
        for _ in 0..2 {
            super::cached_block_overlay(&super::stack_fingerprint(&stack), &blocks, false, || {
                compiles += 1;
                None
            });
        }
        assert_eq!(compiles, 1);
        super::cached_block_overlay(&super::stack_fingerprint(&stack), &blocks, true, || {
            compiles += 1;
            None
        });
        assert_eq!(compiles, 2);
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

#[cfg(test)]
mod fingerprint_bench;
