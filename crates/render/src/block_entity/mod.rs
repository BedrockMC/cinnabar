//! Block-entity rendering: CPU-built model and overlay vertices plus a small wgpu pass.
//!
//! Models are authored in code against the carrier's atlas; break-crack overlays and
//! sign text share the same vertex path. Geometry the pinned pack does not define is
//! marked provisional in its module and needs native measurement.

mod atlas;
mod banner;
mod beam;
mod bed;
mod bell;
mod book;
mod chest;
mod crack;
mod gpu;
mod mesh;
mod portal;
mod scene;
mod shulker;
mod sign;
mod skull;

pub use atlas::{AtlasRect, BlockEntityAtlas, TEXT_CELL, TEXT_SLOT_COUNT, TextureRef};
pub use banner::{
    BannerLayer, BannerModel, BannerMount, MAX_BANNER_LAYERS, banner_color, pattern_texture,
};
pub use beam::BeaconModel;
pub use bed::{BedModel, bed_color};
pub use chest::{ChestModel, ChestPair, ChestVariant, CopperAge, lid_angle_radians};
pub use crack::{CrackQuad, CrackShape, crack_shape_from_template, crack_texture_name};
pub use gpu::BlockEntityRenderPlugin;
pub use mesh::{BLOCK_ENTITY_VERTEX_WORDS, BlockEntityVertex, Facing, MAX_BLOCK_ENTITY_VERTICES};
pub use scene::{
    BlockEntityAtlasImage, BlockEntityFrame, BlockEntityKind, BlockEntityScene,
    BlockEntitySubmission, CrackInstance, SceneClock,
};
pub use shulker::{ShulkerModel, shulker_color_from_block_name};
pub use sign::{SignFace, SignModel, SignMount};
pub use skull::{SkullKind, SkullModel, SkullMount, floor_yaw_degrees};
