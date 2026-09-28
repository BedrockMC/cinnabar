//! Local-world control client, re-exported so the app reaches the bridge through this facade.

pub use bridge::{
    BridgeError, Difficulty, GameMode, Generator, NewWorld, World, WorldState, WorldStatus,
    close_world, create_world, delete_world, list_worlds, open_world, rename_world,
    set_world_paused, world_status,
};
