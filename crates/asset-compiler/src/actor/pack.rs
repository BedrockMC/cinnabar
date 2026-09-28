//! Neutral actor artwork for a server pack's entities, compiled in memory.

use super::*;
use crate::entity::{EntityPackSkips, compile_entity_pack};

/// A pack's entity catalog with the artwork of its eligible rigs. Indices are
/// local to `entities`, which is its own index space beside the vanilla catalog.
#[derive(Debug)]
pub struct ActorPackCompilation {
    pub entities: CompiledEntityAssets,
    pub textures: Vec<ActorTexture>,
    pub bindings: Vec<ActorArtworkBinding>,
    pub skipped: EntityPackSkips,
    /// Rigs left without artwork, with the reason each was rejected.
    pub fallbacks: Vec<ActorFallback>,
}

/// Compiles `(pack-relative path, bytes)` files; `Ok(None)` when the pack has no
/// usable entity source. Bad files are skipped and counted in `skipped`.
pub fn compile_actor_pack(
    files: Vec<(Box<str>, Vec<u8>)>,
) -> Result<Option<ActorPackCompilation>, AssetError> {
    let Some(pack) = compile_entity_pack(files)? else {
        return Ok(None);
    };
    let runtime = assets::RuntimeEntityAssets::from_compiled(pack.assets.clone())?;
    let mut read = |index: u32| -> Result<Vec<u8>, AssetError> {
        let source = &pack.assets.sources[index as usize];
        pack.payloads
            .get(source.path.as_ref())
            .map(|bytes| bytes.to_vec())
            .ok_or_else(|| invalid("pack entity source payload is absent"))
    };
    let build = build_artwork(&pack.assets, &runtime, &mut read)?;
    Ok(Some(ActorPackCompilation {
        entities: pack.assets,
        textures: build.textures,
        bindings: build.bindings,
        skipped: pack.skipped,
        fallbacks: build.fallbacks,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_pack_without_entity_sources_compiles_to_nothing() {
        assert!(
            compile_actor_pack(vec![("textures/blocks/a.png".into(), vec![1])])
                .unwrap()
                .is_none()
        );
    }
}
