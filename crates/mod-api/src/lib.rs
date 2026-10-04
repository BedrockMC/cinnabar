//! Guest SDK for player mods, generated from the same `extension` WIT contract used by the host.
//! A server Experience's client part uses `experience-sdk`'s `client` feature instead.

/// Ticks in one Bedrock day, shared by capability validation and sky math.
pub const BEDROCK_DAY_TICKS: u32 = 24_000;
/// Maximum remote players exposed by one local gameplay snapshot.
pub const MAX_GAMEPLAY_PLAYERS: usize = 128;
/// Maximum accumulated camera change per axis in one callback, in radians.
pub const MAX_CAMERA_DELTA_RADIANS: f32 = 0.25;

pub mod bindings {
    wit_bindgen::generate!({
        path: "wit",
        world: "extension",
        pub_export_macro: true,
    });
}
