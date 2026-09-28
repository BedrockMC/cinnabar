//! Data-driven Bedrock particle engine: effect definitions, Molang-driven emitters, world
//! collision, and instanced billboard rendering.

mod atlas;
mod def;
mod draw;
mod emitter;
mod library;
mod molang;
mod particle;
mod render;
mod system;
mod triggers;
mod world;

pub use atlas::{ATLAS_SIDE, ParticleAtlas, Placement};
pub use draw::{DrawLists, MAX_DRAW_DISTANCE, ParticleInstance, ParticleView};
pub use emitter::{ParticleSound, SpawnRequest, TileRequest};
pub use render::{ParticleGpuFrame, ParticleRenderPlugin, particle_view, update_particle_frame};
pub use system::{MAX_EMITTERS, MAX_LIVE_PARTICLES, MAX_SPAWN_DISTANCE, ParticleSystem};
pub use triggers::{
    BLOCK_BREAK_PARTICLES, BLOCK_CRACK_PARTICLES, LEVEL_EVENT_PARTICLE_FLAG, LevelParticle,
    block_break_request, block_crack_request, classify_level_event, is_particle_level_event,
    legacy_particle_effect, named_request, parse_molang_variables,
};
pub use world::{EmptyWorld, Fluid, ParticleWorld};
