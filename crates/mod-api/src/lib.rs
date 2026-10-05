//! Guest SDK generated from the same WIT contract used by the host.

/// Ticks in one Bedrock day, shared by capability validation and sky math.
pub const BEDROCK_DAY_TICKS: u32 = 24_000;
/// Maximum remote players exposed by one local gameplay snapshot.
pub const MAX_GAMEPLAY_PLAYERS: usize = 128;
/// Maximum accumulated camera change per axis in one callback, in radians.
pub const MAX_CAMERA_DELTA_RADIANS: f32 = 0.25;
/// Personal attack overrides never extend actor selection beyond this local bound.
pub const MAX_ENTITY_REACH_BLOCKS: f32 = 6.0;
/// Bounded control and settings payloads for personal components.
pub const MAX_SETTINGS_BYTES: usize = 16 * 1024;
pub const MAX_CONTROL_KEYS: usize = 64;
/// Maximum bytes in a physical key name supplied by local controls.
pub const MAX_CONTROL_KEY_BYTES: usize = 32;

/// Render capability budgets, per instance or per committed callback.
pub const MAX_RENDER_PASSES: usize = 8;
/// Shader validations allowed in one frame callback; `init` may compile every pass.
pub const MAX_PASS_COMPILES_PER_FRAME: u32 = 1;
pub const MAX_PASS_NAME_BYTES: usize = 32;
pub const MAX_PASS_PARAMS: usize = 16;
pub const MAX_SHADER_BYTES: usize = 16 * 1024;
/// Worst-case texture reads and IR expressions one fragment may execute, helpers included.
pub const MAX_SHADER_TEXTURE_SAMPLES: u32 = 32;
pub const MAX_SHADER_EXPRESSIONS: u32 = 4096;
/// Bounds expression nesting, which is otherwise one level per operator in a statement.
pub const MAX_STATEMENT_TOKENS: usize = 512;
/// Largest value any shader type may hold, which bounds per-pixel private memory.
pub const MAX_SHADER_TYPE_BYTES: u32 = 1024;
pub const MAX_RENDER_DECALS: usize = 64;
pub const MAX_RENDER_RIBBONS: usize = 32;
pub const MAX_RIBBON_POINTS: usize = 64;
pub const MAX_RENDER_BEAMS: usize = 16;
pub const MAX_RENDER_BILLBOARDS: usize = 512;
/// Largest decal radius, ribbon or beam width, or billboard side, in blocks.
pub const MAX_PRIMITIVE_EXTENT_BLOCKS: f32 = 64.0;
/// Primitives farther than this from the origin of either axis are rejected.
pub const MAX_PRIMITIVE_COORDINATE: f32 = 30_000_000.0;

pub mod bindings {
    wit_bindgen::generate!({
        path: "wit",
        world: "extension",
        pub_export_macro: true,
    });
}

/// Versioned SDK for consented server components, separate from personal mods.
pub mod server_bundle {
    wit_bindgen::generate!({
        path: "wit",
        world: "server-bundle",
        generate_all,
        pub_export_macro: true,
        export_macro_name: "export_server_bundle",
    });
}
