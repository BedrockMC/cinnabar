//! Coordinates optional atlas preparation before the main world publishes new material IDs.
use super::{ChunkTextureAssetIdentity, ChunkTextureAssets};
use bevy::{prelude::Resource, render::extract_resource::ExtractResource};
use std::sync::{Arc, Mutex};

#[derive(Default)]
struct State {
    requested: Option<ChunkTextureAssets>,
    result: Option<Result<ChunkTextureAssetIdentity, String>>,
}

/// Shared main/render-world mailbox; GPU replacements remain staged until CPU publication.
#[derive(Resource, Clone, Default, ExtractResource)]
pub struct ChunkTextureReload(Arc<Mutex<State>>);

impl ChunkTextureReload {
    /// Requests one immutable candidate, replacing any obsolete pending request.
    pub fn request(&self, assets: ChunkTextureAssets) {
        let mut state = self.0.lock().unwrap_or_else(|error| error.into_inner());
        if state
            .requested
            .as_ref()
            .is_none_or(|current| current.identity() != assets.identity())
        {
            state.requested = Some(assets);
            state.result = None;
        }
    }

    /// Reports whether this candidate is fully built, without blocking either world.
    pub fn status(&self, identity: ChunkTextureAssetIdentity) -> Option<Result<(), String>> {
        let state = self.0.lock().unwrap_or_else(|error| error.into_inner());
        if state
            .requested
            .as_ref()
            .is_none_or(|assets| assets.identity() != identity)
        {
            return None;
        }
        state
            .result
            .as_ref()
            .map(|result| result.as_ref().map(|_| ()).map_err(Clone::clone))
    }

    /// Retires abandoned requests while preserving an atlas awaiting publication extraction.
    pub fn cancel_except(&self, published: ChunkTextureAssetIdentity) {
        let mut state = self.0.lock().unwrap_or_else(|error| error.into_inner());
        if state
            .requested
            .as_ref()
            .is_some_and(|assets| assets.identity() != published)
        {
            state.requested = None;
            state.result = None;
        }
    }

    /// Clones the candidate for the render preparation worker.
    pub(in crate::chunk) fn requested(&self) -> Option<ChunkTextureAssets> {
        self.0
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .requested
            .clone()
    }

    /// Completes only the currently requested generation; superseded results retire on drop.
    pub(in crate::chunk) fn finish(&self, identity: ChunkTextureAssetIdentity, succeeded: bool) {
        let mut state = self.0.lock().unwrap_or_else(|error| error.into_inner());
        if state
            .requested
            .as_ref()
            .is_some_and(|assets| assets.identity() == identity)
        {
            state.result = Some(if succeeded {
                Ok(identity)
            } else {
                Err("Resource pack atlas exceeds GPU limits or could not be prepared".into())
            });
        }
    }
}

#[cfg(test)]
#[path = "texture_reload_tests.rs"]
mod tests;
