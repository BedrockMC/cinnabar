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
