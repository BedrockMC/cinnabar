use std::{
    fs::{self, File},
    io::{self, Read},
    path::{Path, PathBuf},
};

use asset_compiler::{
    AnimationInventory, AtmosphereCompileOptions, CompileReferenceOutcome, FontCompileError,
    compile_atmosphere_assets_with_options, compile_entity_assets_with_report, compile_fonts,
    compile_pack_with_material_keys, inspect_animation_inventory,
};
use assets::{
    AssetError, AtmosphereRole, BlobProvenance, EntityAssetSource, EntityAssetSymbol,
    ItemVisualDefinitionRoute, MATERIAL_FLAG_ALPHA_CUTOUT, encode_atmosphere_blob, encode_blob,
    encode_entity_blob, read_biome_registry, write_blob_atomic,
};
use clap::{Parser, Subcommand};
use serde::Serialize;
use sha2::{Digest, Sha256};

#[path = "assetc/actor_command.rs"]
mod actor_command;
#[path = "assetc/audio_command.rs"]
mod audio_command;
#[path = "assetc/audio_pcm_command.rs"]
mod audio_pcm_command;
#[path = "assetc/block_entity_command.rs"]
mod block_entity_command;
#[path = "assetc/equipment_command.rs"]
mod equipment_command;
#[path = "assetc/font_command.rs"]
mod font_command;
#[path = "assetc/hud_command.rs"]
mod hud_command;
#[path = "assetc/icon_command.rs"]
mod icon_command;
#[path = "assetc/lang_command.rs"]
mod lang_command;
#[path = "assetc/output_validation.rs"]
mod output_validation;
#[path = "assetc/particle_command.rs"]
mod particle_command;
#[path = "assetc/registry_version.rs"]
mod registry_version;
#[path = "assetc/ui_command.rs"]
mod ui_command;

use audio_command::compile_audio_assets_command;
use audio_pcm_command::compile_audio_pcm_command;
use equipment_command::compile_equipment_assets_command;
use hud_command::compile_hud_assets_command;
use icon_command::compile_icon_assets_command;
use lang_command::compile_lang_assets_command;
use output_validation::validate_output_bundle;
use particle_command::compile_particle_assets_command;
use ui_command::compile_ui_assets_command;

const MAX_REGISTRY_FILE_BYTES: usize = 128 * 1024 * 1024;
const MAX_SOURCE_MANIFEST_BYTES: usize = 1024 * 1024;
#[derive(Debug, Parser)]
#[command(
    about = "Compile verified local Bedrock resource-pack assets",
    after_help = "Compile inputs:\n  assetc compile --pack <RESOURCE_PACK> --source-manifest <VANILLA_SOURCE_JSON> --registry <BLOCK_REGISTRY_BIN> --light-registry <LIGHT_REGISTRY_BIN> --biome-registry <BIOME_REGISTRY_BIN> --out <IGNORED_DIR>/vanilla-v2168.mcbea\n\nAtmosphere inputs:\n  assetc atmosphere --pack <RESOURCE_PACK> --source-manifest <VANILLA_SOURCE_JSON> --out <IGNORED_DIR>/vanilla-v1.mcbeatm --report <IGNORED_DIR>/atmosphere-assets.json\n\nEntity catalog and geometry payloads:\n  assetc entity-assets --pack <RESOURCE_PACK> --source-manifest <VANILLA_SOURCE_JSON> --out <IGNORED_DIR>/vanilla-v1.mcbeent --report <IGNORED_DIR>/entity-assets.json\n\nDormant sound-definition lookup:\n  assetc audio-assets --pack <RESOURCE_PACK> --source-manifest <VANILLA_SOURCE_JSON> --out <IGNORED_DIR>/vanilla-v1.mcbeaud --report <IGNORED_DIR>/audio-assets.json\n\nBitmap font payloads:\n  assetc font-assets --pack <RESOURCE_PACK> --source-manifest <VANILLA_SOURCE_JSON> --out <IGNORED_DIR>/vanilla-v1.mcbefont --report <IGNORED_DIR>/font-assets.json\n\nPinned official Mojang sample HUD sprites:\n  assetc hud-assets --pack <RESOURCE_PACK> --source-manifest assets/hud-source-v1001.json --out <IGNORED_DIR>/vanilla-v1.mcbehud --report <IGNORED_DIR>/hud-assets.json\n\nJSON-UI atlas, sidecars, and raw ui json:\n  assetc ui-assets --pack <RESOURCE_PACK> --source-manifest <VANILLA_SOURCE_JSON> --out <IGNORED_DIR>/vanilla-v1.mcbeui --report <IGNORED_DIR>/ui-assets.json\n\nParticle effects and textures:\n  assetc particle-assets --pack <RESOURCE_PACK> --source-manifest <VANILLA_SOURCE_JSON> --out <IGNORED_DIR>/vanilla-v1.mcbept --report <IGNORED_DIR>/particle-assets.json\n\nAnimation inventory:\n  assetc animation-inventory --pack <RESOURCE_PACK> --source-manifest <VANILLA_SOURCE_JSON> --max-layers-per-page 2048 --max-pages 2 --out <IGNORED_DIR>/animation-inventory.json"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Compile the fixed vanilla sun, moon-phase, and cloud textures.
    Atmosphere {
        /// Root of the pinned vanilla resource pack.
        #[arg(long)]
        pack: PathBuf,
        /// Tracked manifest that pins the local resource-pack source.
        #[arg(long)]
        source_manifest: PathBuf,
        #[arg(long)]
        clouds_override: Option<PathBuf>,
        /// Ignored/local MCBEATM2 output path.
        #[arg(long)]
        out: PathBuf,
        /// Ignored/local deterministic JSON provenance report path.
        #[arg(long)]
        report: PathBuf,
    },
    /// Compile bounded entity geometry, animation, controller, and texture metadata.
    EntityAssets {
        /// Root of the pinned vanilla resource pack.
        #[arg(long)]
        pack: PathBuf,
        /// Tracked manifest that pins the local resource-pack source.
        #[arg(long)]
        source_manifest: PathBuf,
        /// Ignored/local MCBEENT3 output path.
        #[arg(long)]
        out: PathBuf,
        /// Ignored/local deterministic JSON provenance report path.
        #[arg(long)]
        report: PathBuf,
    },
    /// Compile the pinned pack's attachable equipment bindings into the
    /// equipment carrier, pinned to the sibling entity carrier.
    EquipmentAssets {
        /// Root of the pinned vanilla resource pack.
        #[arg(long)]
        pack: PathBuf,
        /// Tracked manifest that pins the local resource-pack source.
        #[arg(long)]
        source_manifest: PathBuf,
        /// Ignored/local MCBEEQP1 output path.
        #[arg(long)]
        out: PathBuf,
        /// Ignored/local deterministic JSON provenance report path.
        #[arg(long)]
        report: PathBuf,
    },
    /// Compile bounded bitmap-font metrics and raw RGBA8 texture pages.
    FontAssets {
        /// Root of the pinned vanilla resource pack.
        #[arg(long)]
        pack: PathBuf,
        /// Tracked manifest that pins the local resource-pack source.
        #[arg(long)]
        source_manifest: PathBuf,
        /// Ignored/local MCBEFONT1 output path.
        #[arg(long)]
        out: PathBuf,
        /// Ignored/local deterministic JSON provenance report path.
        #[arg(long)]
        report: PathBuf,
    },
    /// Compile the optional precipitation sheet and End sky carrier.
    WeatherAssets {
        #[arg(long)]
        pack: PathBuf,
        #[arg(long)]
        out: PathBuf,
    },
    HudAssets {
        #[arg(long)]
        pack: PathBuf,
        #[arg(long)]
        source_manifest: PathBuf,
        #[arg(long)]
        out: PathBuf,
        #[arg(long)]
        report: PathBuf,
    },
    /// Pack block-entity model textures and the version-pinned inventory into
    /// the block-entity carrier.
    BlockEntityAssets {
        #[arg(long)]
        pack: PathBuf,
        #[arg(long)]
        source_manifest: PathBuf,
        #[arg(long)]
        out: PathBuf,
        #[arg(long)]
        report: PathBuf,
    },
    /// Pack the pinned pack's `textures/ui` sprites into atlas pages and store
    /// the nine-slice sidecars plus the raw `ui/*.json` catalog for the JSON-UI
    /// engine. Not yet wired into startup.
    UiAssets {
        #[arg(long)]
        pack: PathBuf,
        #[arg(long)]
        source_manifest: PathBuf,
        #[arg(long)]
        out: PathBuf,
        #[arg(long)]
        report: PathBuf,
    },
    /// Compile the pack's `particles/*.json` effects and particle textures into the
    /// particle carrier.
    ParticleAssets {
        #[arg(long)]
        pack: PathBuf,
        #[arg(long)]
        source_manifest: PathBuf,
        #[arg(long)]
        out: PathBuf,
        #[arg(long)]
        report: PathBuf,
    },
    /// Compile the pinned pack's sprite-routed item icons into the bounded
    /// icon carrier.
    IconAssets {
        #[arg(long)]
        pack: PathBuf,
        #[arg(long)]
        source_manifest: PathBuf,
        /// Optional checked world carrier for ordinary opaque-cube thumbnails.
        #[arg(long)]
        block_assets: Option<PathBuf>,
        #[arg(long)]
        out: PathBuf,
        #[arg(long)]
        report: PathBuf,
    },
    /// Compile unconditional neutral binary-alpha actor artwork.
    ActorAssets {
        #[arg(long)]
        pack: PathBuf,
        #[arg(long)]
        source_manifest: PathBuf,
        #[arg(long)]
        out: PathBuf,
        #[arg(long)]
        report: PathBuf,
    },
    /// Compile the pinned pack's en_US language table into the bounded
    /// localization carrier.
    LangAssets {
        #[arg(long)]
        pack: PathBuf,
        #[arg(long)]
        source_manifest: PathBuf,
        #[arg(long)]
        out: PathBuf,
        #[arg(long)]
        report: PathBuf,
    },
    /// Compile the pinned vanilla sound definitions into a dormant lookup catalog.
    AudioAssets {
        /// Root of the pinned vanilla resource pack.
        #[arg(long)]
        pack: PathBuf,
        /// Tracked manifest that pins the local resource-pack source.
        #[arg(long)]
        source_manifest: PathBuf,
        /// Ignored/local MCBEAUD1 output path.
        #[arg(long)]
        out: PathBuf,
        /// Ignored/local deterministic JSON provenance report path.
        #[arg(long)]
        report: PathBuf,
    },
    /// Compile one reviewed sample into finite PCM; does not activate playback.
    AudioPcmAssets {
        #[arg(long)]
        pack: PathBuf,
        #[arg(long)]
        catalog: PathBuf,
        #[arg(long)]
        source_manifest: PathBuf,
        #[arg(long)]
        out: PathBuf,
        #[arg(long)]
        report: PathBuf,
    },
    /// Rasterize a pinned open-licensed outline font into a bounded bitmap carrier.
    OutlineFontAssets {
        /// Exact hash-verified local TTF/OTF source.
        #[arg(long)]
        font: PathBuf,
        /// Exact hash-verified secondary source required by a fallback manifest.
        #[arg(long)]
        fallback_font: Option<PathBuf>,
        /// Tracked manifest pinning font URL, hash, license, and raster settings.
        #[arg(long)]
        source_manifest: PathBuf,
        /// Ignored/local MCBEFONT1 output path.
        #[arg(long)]
        out: PathBuf,
        /// Ignored/local deterministic JSON provenance report path.
        #[arg(long)]
        report: PathBuf,
    },
    /// Compile a resource pack and Dragonfly registry into a runtime blob.
    Compile {
        /// Root containing blocks.json and the textures directory.
        #[arg(long)]
        pack: PathBuf,
        /// Tracked manifest that pins the local resource-pack source.
        #[arg(long)]
        source_manifest: PathBuf,
        /// BREG1003 registry exported by tools/registrygen.
        #[arg(long)]
        registry: PathBuf,
        /// LREG1001 state light metadata bound to the exact BREG1003 input.
        #[arg(long)]
        light_registry: PathBuf,
        /// BIOREG01 registry exported by tools/registrygen.
        #[arg(long)]
        biome_registry: PathBuf,
        /// Ignored/local output path, conventionally ending in .mcbea.
        #[arg(long)]
        out: PathBuf,
    },
    /// Compile a bounded read-only animation plan and write its deterministic inventory.
    AnimationInventory {
        /// Root containing blocks.json and the textures directory.
        #[arg(long)]
        pack: PathBuf,
        /// Pinned source manifest whose exact bytes identify the local pack source.
        #[arg(long)]
        source_manifest: PathBuf,
        /// Maximum physical array layers in each texture page (1..=2048).
        #[arg(long)]
        max_layers_per_page: u32,
        /// Maximum physical texture pages (1..=2).
        #[arg(long)]
        max_pages: u32,
        /// Ignored/local deterministic JSON report path.
        #[arg(long)]
        out: PathBuf,
    },
}

#[derive(Serialize)]
struct AnimationInventoryReport {
    schema: u32,
    source_manifest_sha256: Box<str>,
    canonical_pack_path: Box<str>,
    limits: AnimationInventoryLimits,
    inventory: AnimationInventory,
}

#[derive(Serialize)]
struct AnimationInventoryLimits {
    max_layers_per_page: u32,
    max_pages: u32,
}

#[derive(Serialize)]
struct AtmosphereReport {
    schema: u32,
    source: serde_json::Value,
    source_manifest_sha256: Box<str>,
    blob_sha256: Box<str>,
    textures: Box<[AtmosphereTextureReport]>,
}

#[derive(Serialize)]
struct AtmosphereTextureReport {
    role: &'static str,
    source_path: Box<str>,
    width: u32,
    height: u32,
    source_bytes: usize,
    decoded_rgba8_bytes: usize,
    source_sha256: Box<str>,
    pixels_sha256: Box<str>,
}

#[derive(Serialize)]
struct EntityAssetsReport<'a> {
    schema: u32,
    source: serde_json::Value,
    source_manifest_sha256: Box<str>,
    blob_sha256: Box<str>,
    counts: EntityAssetCounts,
    sources: &'a [EntityAssetSource],
    symbols: &'a [EntityAssetSymbol],
    reference_outcomes: &'a [CompileReferenceOutcome<u32>],
}

#[derive(Serialize)]
struct EntityAssetCounts {
    sources: usize,
    symbols: usize,
    dependencies: usize,
    geometries: usize,
    bones: usize,
    cubes: usize,
    animation_clips: usize,
    animation_channels: usize,
    animation_keyframes: usize,
    molang_symbols: usize,
    molang_expressions: usize,
    molang_ops: usize,
    molang_collections: usize,
    molang_collection_items: usize,
    controllers: usize,
    controller_states: usize,
    controller_animations: usize,
    controller_transitions: usize,
    rig_bindings: usize,
    rig_geometry_candidates: usize,
    rig_animations: usize,
    rig_controllers: usize,
    rig_geometry_selections: usize,
    item_visuals: usize,
    item_visual_aliases: usize,
    item_sprite_routes: usize,
    item_block_routes: usize,
    item_empty_hand_routes: usize,
    item_missing_routes: usize,
    block_visuals: usize,
}

#[derive(Serialize)]
struct FontAssetsReport {
    schema: u32,
    source: serde_json::Value,
    source_manifest_sha256: Box<str>,
    carrier_sha256: Box<str>,
    counts: FontAssetCounts,
}

#[derive(Serialize)]
struct FontAssetCounts {
    glyphs: usize,
    pages: usize,
    source_bytes: u64,
    decoded_bytes: u64,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    match Cli::parse().command {
        Command::Atmosphere {
            pack,
            source_manifest,
            clouds_override,
            out,
            report,
        } => {
            compile_atmosphere_command(
                &pack,
                &source_manifest,
                clouds_override.as_deref(),
                &out,
                &report,
                compile_atmosphere_assets_with_options,
            )?;
        }
        Command::EntityAssets {
            pack,
            source_manifest,
            out,
            report,
        } => {
            compile_entity_assets_command(&pack, &source_manifest, &out, &report)?;
        }
        Command::EquipmentAssets {
            pack,
            source_manifest,
            out,
            report,
        } => {
            compile_equipment_assets_command(&pack, &source_manifest, &out, &report)?;
        }
        Command::FontAssets {
            pack,
            source_manifest,
            out,
            report,
        } => {
            compile_font_assets_command(&pack, &source_manifest, &out, &report)?;
        }
        Command::HudAssets {
            pack,
            source_manifest,
            out,
            report,
        } => {
            compile_hud_assets_command(&pack, &source_manifest, &out, &report)?;
        }
        Command::WeatherAssets { pack, out } => {
            asset_compiler::compile_weather_textures_to_file(&pack, &out)?;
            println!("compiled weather textures to {}", out.display());
        }
        Command::ActorAssets {
            pack,
            source_manifest,
            out,
            report,
        } => {
            actor_command::compile_actor_assets_command(&pack, &source_manifest, &out, &report)?;
        }
        Command::BlockEntityAssets {
            pack,
            source_manifest,
            out,
            report,
        } => {
            block_entity_command::compile_block_entity_assets_command(
                &pack,
                &source_manifest,
                &out,
                &report,
            )?;
        }
        Command::UiAssets {
            pack,
            source_manifest,
            out,
            report,
        } => {
            compile_ui_assets_command(&pack, &source_manifest, &out, &report)?;
        }
        Command::ParticleAssets {
            pack,
            source_manifest,
            out,
            report,
        } => {
            compile_particle_assets_command(&pack, &source_manifest, &out, &report)?;
        }
        Command::IconAssets {
            pack,
            source_manifest,
            block_assets,
            out,
            report,
        } => {
            compile_icon_assets_command(
                &pack,
                &source_manifest,
                block_assets.as_deref(),
                &out,
                &report,
            )?;
        }
        Command::LangAssets {
            pack,
            source_manifest,
            out,
            report,
        } => {
            compile_lang_assets_command(&pack, &source_manifest, &out, &report)?;
        }
        Command::AudioAssets {
            pack,
            source_manifest,
            out,
            report,
        } => {
            compile_audio_assets_command(&pack, &source_manifest, &out, &report)?;
        }
        Command::AudioPcmAssets {
            pack,
            catalog,
            source_manifest,
            out,
            report,
        } => {
            compile_audio_pcm_command(&pack, &catalog, &source_manifest, &out, &report)?;
        }
        Command::OutlineFontAssets {
            font,
            fallback_font,
            source_manifest,
            out,
            report,
        } => {
            compile_outline_font_assets_command(
                &font,
                fallback_font.as_deref(),
                &source_manifest,
                &out,
                &report,
            )?;
        }
        Command::Compile {
            pack,
            source_manifest,
            registry,
            light_registry,
            biome_registry,
            out,
        } => {
            let manifest_bytes = read_bounded_with_limit(
                &source_manifest,
                MAX_SOURCE_MANIFEST_BYTES,
                "source manifest",
            )?;
            serde_json::from_slice::<serde_json::Value>(&manifest_bytes).map_err(|source| {
                AssetError::Json {
                    path: source_manifest.clone(),
                    source,
                }
            })?;
            let registry_bytes = read_bounded(&registry)?;
            let (records, block_registry_protocol) =
                registry_version::read_block_registry_input(&registry, &registry_bytes)?;
            let light_registry_bytes = read_bounded(&light_registry)?;
            let light_properties = registry_version::read_light_registry_input(
                &light_registry,
                &light_registry_bytes,
                &registry,
                &registry_bytes,
                block_registry_protocol,
                records.len(),
            )?;
            let biome_registry_bytes = read_bounded(&biome_registry)?;
            let biome_records = read_biome_registry(&biome_registry_bytes)?;
            let behavior_pack = pack
                .parent()
                .ok_or("resource-pack path has no parent for behavior_pack")?
                .join("behavior_pack");
            let (mut compiled, material_keys) = compile_pack_with_material_keys(
                &pack,
                &behavior_pack,
                &records,
                &biome_records,
                &light_properties,
                block_registry_protocol,
            )?;
            compiled.provenance = BlobProvenance {
                source_manifest_sha256: assets::canonical_source_manifest_sha256(&manifest_bytes),
                block_registry_sha256: Sha256::digest(&registry_bytes).into(),
                light_registry_sha256: Sha256::digest(&light_registry_bytes).into(),
                biome_registry_sha256: Sha256::digest(&biome_registry_bytes).into(),
            };
            let blob = encode_blob(&compiled)?;
            write_blob_atomic(&out, &blob)?;
            // Sidecar for runtime retexturing; a stale or absent one only disables that.
            write_blob_atomic(
                &out.with_extension("matkeys.json"),
                &material_keys.to_json(compiled.materials.len() as u32),
            )?;
            let cutout_materials = compiled
                .materials
                .iter()
                .filter(|material| material.flags & MATERIAL_FLAG_ALPHA_CUTOUT != 0)
                .count();
            println!(
                "compiled {} visuals, {} materials ({} alpha cutout), {} texture layers, and {} biome rules to {}",
                compiled.visuals.len(),
                compiled.materials.len(),
                cutout_materials,
                compiled
                    .texture_pages
                    .iter()
                    .map(|page| page.texture.layers)
                    .sum::<u32>(),
                compiled.biomes.rules.len(),
                out.display()
            );
        }
        Command::AnimationInventory {
            pack,
            source_manifest,
            max_layers_per_page,
            max_pages,
            out,
        } => {
            let canonical_pack = fs::canonicalize(&pack).map_err(|source| AssetError::Io {
                path: pack.clone(),
                source,
            })?;
            let manifest_bytes = read_bounded_with_limit(
                &source_manifest,
                MAX_SOURCE_MANIFEST_BYTES,
                "source manifest",
            )?;
            serde_json::from_slice::<serde_json::Value>(&manifest_bytes).map_err(|source| {
                AssetError::Json {
                    path: source_manifest.clone(),
                    source,
                }
            })?;
            let source_manifest_sha256 = format!("{:x}", Sha256::digest(&manifest_bytes));
            let inventory =
                inspect_animation_inventory(&canonical_pack, max_layers_per_page, max_pages)?;
            let report = AnimationInventoryReport {
                schema: 1,
                source_manifest_sha256: source_manifest_sha256.into_boxed_str(),
                canonical_pack_path: canonical_pack
                    .to_string_lossy()
                    .into_owned()
                    .into_boxed_str(),
                limits: AnimationInventoryLimits {
                    max_layers_per_page,
                    max_pages,
                },
                inventory,
            };
            let mut bytes =
                serde_json::to_vec_pretty(&report).map_err(|source| AssetError::Json {
                    path: out.clone(),
                    source,
                })?;
            bytes.push(b'\n');
            write_blob_atomic(&out, &bytes)?;
            println!(
                "inspected {} reachable animations, {} physical frames, {} deduplicated layers across {} pages to {}",
                report.inventory.reachable_animations,
                report.inventory.physical_animation_frames,
                report.inventory.deduplicated_layers,
                report.inventory.pages,
                out.display()
            );
        }
    }
    Ok(())
}

fn compile_font_assets_command(
    pack: &Path,
    source_manifest: &Path,
    out: &Path,
    report: &Path,
) -> Result<(), Box<dyn std::error::Error>> {
    let manifest_bytes = read_bounded_with_limit(
        source_manifest,
        MAX_SOURCE_MANIFEST_BYTES,
        "source manifest",
    )?;
    let source =
        serde_json::from_slice::<serde_json::Value>(&manifest_bytes).map_err(|source| {
            AssetError::Json {
                path: source_manifest.to_path_buf(),
                source,
            }
        })?;
    let source_manifest_sha256 = assets::canonical_source_manifest_sha256(&manifest_bytes);
    let compiled = compile_fonts(pack)?;
    if compiled.report.source_manifest_sha256 != source_manifest_sha256 {
        return Err(FontCompileError::SourceManifestMismatch.into());
    }
    write_compiled_font_assets(source, source_manifest_sha256, compiled, out, report)
}

fn compile_outline_font_assets_command(
    font: &Path,
    fallback: Option<&Path>,
    source_manifest: &Path,
    out: &Path,
    report: &Path,
) -> Result<(), Box<dyn std::error::Error>> {
    font_command::compile(font, fallback, source_manifest, out, report)
}

fn required_u32(value: &serde_json::Value, field: &str) -> Result<u32, Box<dyn std::error::Error>> {
    value
        .get(field)
        .and_then(serde_json::Value::as_u64)
        .and_then(|value| u32::try_from(value).ok())
        .ok_or_else(|| format!("font rasterization field '{field}' is invalid").into())
}

fn write_compiled_font_assets(
    source: serde_json::Value,
    source_manifest_sha256: [u8; 32],
    compiled: asset_compiler::CompiledFontCarrier,
    out: &Path,
    report: &Path,
) -> Result<(), Box<dyn std::error::Error>> {
    if compiled.report.source_manifest_sha256 != source_manifest_sha256 {
        return Err(FontCompileError::SourceManifestMismatch.into());
    }
    let report_data = FontAssetsReport {
        schema: compiled.report.schema,
        source,
        source_manifest_sha256: hex(&compiled.report.source_manifest_sha256).into_boxed_str(),
        carrier_sha256: hex(&compiled.report.carrier_sha256).into_boxed_str(),
        counts: FontAssetCounts {
            glyphs: compiled.report.glyphs,
            pages: compiled.report.pages,
            source_bytes: compiled.report.source_bytes,
            decoded_bytes: compiled.report.decoded_bytes,
        },
    };
    let mut report_bytes =
        serde_json::to_vec_pretty(&report_data).map_err(|source| AssetError::Json {
            path: report.to_path_buf(),
            source,
        })?;
    report_bytes.push(b'\n');
    validate_output_bundle(out, report)?;
    write_blob_atomic(out, &compiled.bytes)?;
    write_blob_atomic(report, &report_bytes)?;
    println!(
        "compiled {} bitmap-font glyphs across {} pages to {} and {}",
        report_data.counts.glyphs,
        report_data.counts.pages,
        out.display(),
        report.display()
    );
    Ok(())
}

fn compile_entity_assets_command(
    pack: &Path,
    source_manifest: &Path,
    out: &Path,
    report: &Path,
) -> Result<(), Box<dyn std::error::Error>> {
    let manifest_bytes = read_bounded_with_limit(
        source_manifest,
        MAX_SOURCE_MANIFEST_BYTES,
        "source manifest",
    )?;
    let source =
        serde_json::from_slice::<serde_json::Value>(&manifest_bytes).map_err(|source| {
            AssetError::Json {
                path: source_manifest.to_path_buf(),
                source,
            }
        })?;
    let mut compilation = compile_entity_assets_with_report(pack, &manifest_bytes)?;
    compilation.reference_outcomes.sort_by_key(outcome_sort_key);
    let compiled = &compilation.assets;
    let blob = encode_entity_blob(compiled)?;
    let report_data = EntityAssetsReport {
        schema: 4,
        source,
        source_manifest_sha256: hex(&compiled.source_manifest_sha256).into_boxed_str(),
        blob_sha256: format!("{:x}", Sha256::digest(&blob)).into_boxed_str(),
        counts: EntityAssetCounts {
            sources: compiled.sources.len(),
            symbols: compiled.symbols.len(),
            dependencies: compiled
                .symbols
                .iter()
                .map(|symbol| symbol.dependencies.len())
                .sum(),
            geometries: compiled.geometries.len(),
            bones: compiled
                .geometries
                .iter()
                .map(|geometry| geometry.bones.len())
                .sum(),
            cubes: compiled
                .geometries
                .iter()
                .flat_map(|geometry| geometry.bones.iter())
                .map(|bone| bone.cubes.len())
                .sum(),
            animation_clips: compiled.animation_clips.len(),
            animation_channels: compiled.animation_channels.len(),
            animation_keyframes: compiled.animation_keyframes.len(),
            molang_symbols: compiled.molang_symbols.len(),
            molang_expressions: compiled.molang_expressions.len(),
            molang_ops: compiled.molang_ops.len(),
            molang_collections: compiled.molang_collections.len(),
            molang_collection_items: compiled.molang_collection_items.len(),
            controllers: compiled.controllers.len(),
            controller_states: compiled.controller_states.len(),
            controller_animations: compiled.controller_animations.len(),
            controller_transitions: compiled.controller_transitions.len(),
            rig_bindings: compiled.rig_bindings.len(),
            rig_geometry_candidates: compiled.rig_geometries.len(),
            rig_animations: compiled.rig_animations.len(),
            rig_controllers: compiled.rig_controllers.len(),
            rig_geometry_selections: compiled
                .rig_geometries
                .iter()
                .filter(|candidate| candidate.condition.is_some())
                .count(),
            item_visuals: compiled.item_visuals.len(),
            item_visual_aliases: compiled.item_visual_aliases.len(),
            item_sprite_routes: compiled
                .item_visuals
                .iter()
                .filter(|visual| matches!(visual.route, ItemVisualDefinitionRoute::Sprite { .. }))
                .count(),
            item_block_routes: compiled
                .item_visuals
                .iter()
                .filter(|visual| {
                    matches!(visual.route, ItemVisualDefinitionRoute::BlockItem { .. })
                })
                .count(),
            item_empty_hand_routes: compiled
                .item_visuals
                .iter()
                .filter(|visual| matches!(visual.route, ItemVisualDefinitionRoute::EmptyHand))
                .count(),
            item_missing_routes: compiled
                .item_visuals
                .iter()
                .filter(|visual| matches!(visual.route, ItemVisualDefinitionRoute::Missing))
                .count(),
            block_visuals: compiled.block_visual_count as usize,
        },
        sources: &compiled.sources,
        symbols: &compiled.symbols,
        reference_outcomes: &compilation.reference_outcomes,
    };
    let mut report_bytes =
        serde_json::to_vec_pretty(&report_data).map_err(|source| AssetError::Json {
            path: report.to_path_buf(),
            source,
        })?;
    report_bytes.push(b'\n');
    validate_output_bundle(out, report)?;
    write_blob_atomic(out, &blob)?;
    write_blob_atomic(report, &report_bytes)?;
    println!(
        "compiled {} entity authority sources, {} symbols, {} dependencies, {} geometries, {} bones, and {} cubes to {} and {}",
        report_data.counts.sources,
        report_data.counts.symbols,
        report_data.counts.dependencies,
        report_data.counts.geometries,
        report_data.counts.bones,
        report_data.counts.cubes,
        out.display(),
        report.display()
    );
    Ok(())
}

fn outcome_sort_key(outcome: &CompileReferenceOutcome<u32>) -> (u32, u32, u8, u8) {
    match outcome {
        CompileReferenceOutcome::Resolved(index) => (u32::MAX, *index, 0, 0),
        CompileReferenceOutcome::OptionalStaticFallback {
            source,
            symbol,
            reason,
        } => (*source, *symbol, 1, *reason as u8),
        CompileReferenceOutcome::RequiredRigRejected {
            source,
            symbol,
            reason,
        } => (*source, *symbol, 2, *reason as u8),
    }
}

fn compile_atmosphere_command<F>(
    pack: &Path,
    source_manifest: &Path,
    clouds_override: Option<&Path>,
    out: &Path,
    report: &Path,
    compile: F,
) -> Result<(), Box<dyn std::error::Error>>
where
    F: for<'a> FnOnce(
        &Path,
        &[u8],
        AtmosphereCompileOptions<'a>,
    ) -> Result<assets::CompiledAtmosphereAssets, AssetError>,
{
    let manifest_bytes = read_bounded_with_limit(
        source_manifest,
        MAX_SOURCE_MANIFEST_BYTES,
        "source manifest",
    )?;
    let source =
        serde_json::from_slice::<serde_json::Value>(&manifest_bytes).map_err(|source| {
            AssetError::Json {
                path: source_manifest.to_path_buf(),
                source,
            }
        })?;
    let compiled = compile(
        pack,
        &manifest_bytes,
        AtmosphereCompileOptions { clouds_override },
    )?;
    let blob = encode_atmosphere_blob(&compiled)?;
    let report_data = build_atmosphere_report(source, &compiled, &blob);
    let mut report_bytes =
        serde_json::to_vec_pretty(&report_data).map_err(|source| AssetError::Json {
            path: report.to_path_buf(),
            source,
        })?;
    report_bytes.push(b'\n');
    validate_output_bundle(out, report)?;
    write_blob_atomic(out, &blob)?;
    write_blob_atomic(report, &report_bytes)?;
    println!(
        "compiled {} pinned atmosphere textures to {} and {}",
        report_data.textures.len(),
        out.display(),
        report.display()
    );
    Ok(())
}

fn build_atmosphere_report(
    source: serde_json::Value,
    compiled: &assets::CompiledAtmosphereAssets,
    blob: &[u8],
) -> AtmosphereReport {
    let textures = compiled
        .textures
        .iter()
        .map(|texture| AtmosphereTextureReport {
            role: match texture.role {
                AtmosphereRole::Sun => "sun",
                AtmosphereRole::MoonPhases => "moon_phases",
                AtmosphereRole::Clouds => "clouds",
            },
            source_path: texture.source_path.clone(),
            width: texture.width,
            height: texture.height,
            source_bytes: texture.source_bytes as usize,
            decoded_rgba8_bytes: texture.rgba8.len(),
            source_sha256: hex(&texture.source_sha256).into_boxed_str(),
            pixels_sha256: hex(&texture.pixels_sha256).into_boxed_str(),
        })
        .collect::<Vec<_>>()
        .into_boxed_slice();
    AtmosphereReport {
        schema: 1,
        source,
        source_manifest_sha256: hex(&compiled.source_manifest_sha256).into_boxed_str(),
        blob_sha256: format!("{:x}", Sha256::digest(blob)).into_boxed_str(),
        textures,
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn read_bounded(path: &Path) -> Result<Vec<u8>, AssetError> {
    read_bounded_with_limit(path, MAX_REGISTRY_FILE_BYTES, "registry")
}

fn read_bounded_with_limit(
    path: &Path,
    max_bytes: usize,
    label: &'static str,
) -> Result<Vec<u8>, AssetError> {
    let file = File::open(path).map_err(|source| AssetError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    let mut bytes = Vec::new();
    file.take((max_bytes + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|source| AssetError::Io {
            path: path.to_path_buf(),
            source,
        })?;
    if bytes.len() > max_bytes {
        return Err(AssetError::Io {
            path: path.to_path_buf(),
            source: io::Error::new(
                io::ErrorKind::InvalidData,
                format!("{label} exceeds the {max_bytes}-byte compiler input limit"),
            ),
        });
    }
    Ok(bytes)
}

#[cfg(test)]
#[path = "assetc/tests.rs"]
mod tests;
