//! Guest SDK generated from the same WIT contract used by the host.

/// Ticks in one Bedrock day, shared by capability validation and sky math.
pub const BEDROCK_DAY_TICKS: u32 = 24_000;
/// Maximum remote players exposed by one local gameplay snapshot.
pub const MAX_GAMEPLAY_PLAYERS: usize = 128;
/// Maximum accumulated camera change per axis in one callback, in radians.
pub const MAX_CAMERA_DELTA_RADIANS: f32 = 0.25;
/// Personal attack overrides never extend actor selection beyond this local bound.
pub const MAX_ENTITY_REACH_BLOCKS: f32 = 6.0;
/// Maximum opt-in delay of post-login application packets in either direction.
pub const MAX_PACKET_DELAY_MS: u32 = 1_000;
/// Bounded control and settings payloads for personal components.
pub const MAX_SETTINGS_BYTES: usize = 16 * 1024;
pub const MAX_CONTROL_KEYS: usize = 64;
/// Maximum bytes in a physical key name supplied by local controls.
pub const MAX_CONTROL_KEY_BYTES: usize = 32;
/// Maximum nearby mobs in one snapshot, and the radius they are drawn from.
pub const MAX_GAMEPLAY_MOBS: usize = 64;
pub const MAX_MOB_RANGE_BLOCKS: f32 = 64.0;
pub const MAX_MOB_TYPE_BYTES: usize = 64;
/// Camera rig bounds: |right| and up/down offset, boom length, |roll| and |FOV change|.
pub const MAX_RIG_SIDE_BLOCKS: f32 = 2.0;
pub const MAX_RIG_VERTICAL_BLOCKS: f32 = 2.0;
pub const MAX_RIG_BACK_BLOCKS: f32 = 8.0;
pub const MAX_RIG_ROLL_RADIANS: f32 = 0.6;
pub const MAX_RIG_FOV_DELTA_DEGREES: f32 = 30.0;
/// Granted command names, their byte bound, and per-frame command requests.
pub const MAX_COMMAND_GRANTS: usize = 8;
pub const MAX_COMMAND_BYTES: usize = 128;
pub const MAX_COMMANDS_PER_FRAME: usize = 4;
/// Command requests accepted per second of gameplay frame time.
pub const MAX_COMMANDS_PER_SECOND: usize = 10;
/// Presentation cue bounds per frame.
pub const MAX_CUES_PER_FRAME: usize = 16;
pub const MAX_CUE_NAME_BYTES: usize = 32;
pub const MAX_CUE_VALUES: usize = 8;
/// Cues delivered to one callback from every loaded mod.
pub const MAX_INCOMING_CUES: usize = 64;

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
