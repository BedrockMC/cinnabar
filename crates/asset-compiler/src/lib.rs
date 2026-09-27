mod animation;
mod atmosphere;
mod audio;
mod audio_pcm;
mod biome;
mod compiler;
mod entity;
mod fadpcm;
mod font;
mod hud;
mod icon;
mod image;
mod lang;
mod pack;

pub use animation::AnimationInventory;
pub use assets::BlockFace;
pub use atmosphere::{
    AtmosphereCompileOptions, compile_atmosphere_assets, compile_atmosphere_assets_with_options,
};
pub use audio::{
    AUDIO_SOUND_DEFINITIONS_RELATIVE_PATH, AudioCompileError, AudioCompileReport,
    CompiledAudioCarrier, PINNED_SOUND_DEFINITIONS_SHA256, compile_audio_assets,
};
pub use audio_pcm::{
    AudioPcmCompileError, AudioPcmCompileReport, CompiledAudioPcmCarrier, compile_audio_pcm_assets,
};
pub use biome::compile_biome_assets;
pub use compiler::{compile_pack, compile_pack_with_biomes, inspect_animation_inventory};
pub use entity::{
    CompileReferenceOutcome, EntityAssetCompilation, FallbackReason, RejectReason,
    compile_entity_assets, compile_entity_assets_with_report,
};
pub use fadpcm::{DecodedFadpcm, FadpcmDecodeError, decode_fsb5_fadpcm};
pub use font::{
    CompiledFontCarrier, FontCompileError, FontCompileReport, GlyphAdvances, OutlineFontConfig,
    compile_fonts, compile_outline_font, compile_outline_font_with_fallback,
};
pub use hud::{CompiledHudCarrier, HudCompileError, HudCompileReport, compile_hud_assets};
pub use icon::{CompiledIconCarrier, IconCompileReport, compile_icon_assets};
pub use lang::{CompiledLangCarrier, LangCompileError, LangCompileReport, compile_lang_assets};
pub use pack::{
    BlockTextureMap, FlipbookSource, MAX_FLIPBOOK_FRAMES, MAX_FLIPBOOKS, PackSources,
    TerrainTextureMap, TextureKey, read_pack, resolve_texture_key,
};
