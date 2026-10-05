//! Immutable server-pack actor artwork shared by compilation and presentation.
use crate::{
    ActorArtworkBinding, ActorTexture, AssetError, RuntimeActorCatalog, RuntimeEntityAssets,
    RuntimeEquipmentCatalog,
};
use std::sync::Arc;
/// The pack's entity catalog with the artwork of its eligible rigs.
#[derive(Debug)]
pub struct SessionEntityPack {
    pub assets: Arc<RuntimeEntityAssets>,
    pub textures: Arc<[ActorTexture]>,
    pub bindings: Arc<[ActorArtworkBinding]>,
    /// The pack's attachable bindings and rasters for held and worn items.
    pub equipment: Option<Arc<crate::RuntimeEquipmentCatalog>>,
}

impl SessionEntityPack {
    /// Bytes [`Self::decode`] reads back; `entity_blob` is the encoding `assets` was built from.
    pub fn encode(&self, entity_blob: &[u8]) -> Result<Vec<u8>, AssetError> {
        let actor = crate::encode_actor_catalog(entity_blob, &self.textures, &self.bindings)?;
        let equipment = self
            .equipment
            .as_deref()
            .map(|catalog| {
                crate::encode_equipment_catalog_with_textures(
                    catalog.source_manifest_sha256(),
                    catalog.entity_blob_sha256(),
                    catalog.bindings(),
                    catalog.textures(),
                )
            })
            .transpose()?;
        let mut bytes = Vec::new();
        for section in [Some(entity_blob), Some(actor.as_slice()), equipment.as_deref()] {
            let Some(section) = section else { continue };
            bytes.extend_from_slice(&(section.len() as u64).to_le_bytes());
            bytes.extend_from_slice(section);
        }
        Ok(bytes)
    }

    /// Decodes and validates each section with its carrier's own checks.
    pub fn decode(bytes: &[u8]) -> Result<Self, AssetError> {
        let mut rest = bytes;
        let mut section = || -> Result<Option<&[u8]>, AssetError> {
            if rest.is_empty() {
                return Ok(None);
            }
            let invalid = || AssetError::InvalidCompiledAssets {
                detail: "session entity pack section is truncated".into(),
            };
            let (length, tail) = rest.split_at_checked(8).ok_or_else(invalid)?;
            let length = usize::try_from(u64::from_le_bytes(length.try_into().unwrap()))
                .map_err(|_| invalid())?;
            let (body, tail) = tail.split_at_checked(length).ok_or_else(invalid)?;
            rest = tail;
            Ok(Some(body))
        };
        let missing = || AssetError::InvalidCompiledAssets {
            detail: "session entity pack section is missing".into(),
        };
        let entity_blob = section()?.ok_or_else(missing)?;
        let actor = RuntimeActorCatalog::decode(section()?.ok_or_else(missing)?, entity_blob)?;
        let equipment = section()?
            .map(RuntimeEquipmentCatalog::decode)
            .transpose()?
            .map(Arc::new);
        if section()?.is_some() {
            return Err(AssetError::InvalidCompiledAssets {
                detail: "session entity pack has trailing bytes".into(),
            });
        }
        Ok(Self {
            assets: Arc::new(RuntimeEntityAssets::decode(entity_blob)?),
            textures: actor.textures().into(),
            bindings: actor.bindings().into(),
            equipment,
        })
    }
}
