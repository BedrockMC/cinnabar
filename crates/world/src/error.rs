use thiserror::Error;

use crate::BlockEntityError;

/// The process-wide collision revision identity space has been exhausted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum CollisionRevisionError {
    #[error("collision revision identity space is exhausted")]
    Exhausted,
}

/// Errors produced while committing decoded chunk data.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum DecodeError {
    #[error(transparent)]
    CollisionRevision(#[from] CollisionRevisionError),

    #[error(transparent)]
    BlockEntity(#[from] BlockEntityError),
}

/// Errors produced before mutating packed block storage.
///
/// All updates in a batch are validated before the store is changed, so these
/// errors never leave a partially-applied sub-chunk behind.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum MutationError {
    #[error(transparent)]
    CollisionRevision(#[from] CollisionRevisionError),

    #[error("local block coordinates ({x}, {y}, {z}) are outside a 16x16x16 sub-chunk")]
    LocalCoordinatesOutOfBounds { x: u8, y: u8, z: u8 },

    #[error("block storage layer {layer} exceeds the client limit of {max}")]
    LayerOutOfBounds { layer: u32, max: usize },
}
