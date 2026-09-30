//! Bounded neutral actor artwork. This is not a complete retail material model.
use std::sync::Arc;

use sha2::{Digest, Sha256};

use crate::{AssetError, RuntimeEntityAssets};
mod eligibility;
pub use eligibility::neutral_actor_geometry_uvs_are_supported;

pub const ACTOR_CARRIER_MAGIC: [u8; 8] = *b"MCBEACT3";
pub const ACTOR_CARRIER_VERSION: u32 = 3;
// Engine safety ceilings, not retail constants.
pub const MAX_ACTOR_TEXTURE_SIDE: u16 = 512;
pub const MAX_ACTOR_TEXTURES: usize = 2048;
pub const MAX_ACTOR_BINDINGS: usize = 4096;
pub const MAX_ACTOR_PIXEL_BYTES: usize = 32 * 1024 * 1024;
pub const MAX_ACTOR_CARRIER_BYTES: usize = MAX_ACTOR_PIXEL_BYTES + 1024 * 1024;
const HEADER: usize = 128;
const HASH: usize = 32;
const POLICY: &[u8] = include_bytes!("../data/neutral-actor-materials-v1.json");

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ActorTexture {
    pub source: u32,
    pub width: u16,
    pub height: u16,
    pub pixel_sha256: [u8; 32],
    pub rgba8: Arc<[u8]>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ActorArtworkBinding {
    pub rig: u32,
    pub geometry_candidate: u32,
    pub entity_symbol: u32,
    pub geometry: u32,
    pub render_controller: u32,
    pub texture: u32,
    pub material: Box<str>,
    pub pose_mode: ActorPoseMode,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize)]
#[repr(u32)]
pub enum ActorPoseMode {
    CompiledLiteral = 0,
    RestPose = 1,
}
impl ActorPoseMode {
    pub fn reason(self) -> &'static str {
        match self {
            Self::CompiledLiteral => "literal_pose_only",
            Self::RestPose => "pose_expression_unverified",
        }
    }
}
pub use eligibility::neutral_actor_pose_mode;

#[derive(Debug)]
pub struct RuntimeActorCatalog {
    identity: [u8; 32],
    entity_identity: [u8; 32],
    textures: Arc<[ActorTexture]>,
    bindings: Arc<[ActorArtworkBinding]>,
}

pub fn neutral_actor_material_is_supported(name: &str) -> bool {
    #[derive(serde::Deserialize)]
    struct Policy {
        material_names: Vec<Box<str>>,
    }
    serde_json::from_slice::<Policy>(POLICY).is_ok_and(|policy| {
        policy
            .material_names
            .iter()
            .any(|value| value.as_ref() == name)
    })
}

impl RuntimeActorCatalog {
    pub fn decode(bytes: &[u8], entity_bytes: &[u8]) -> Result<Self, AssetError> {
        if bytes.len() < HEADER + HASH || bytes.len() > MAX_ACTOR_CARRIER_BYTES {
            return Err(invalid("actor carrier size exceeds bounds"));
        }
        let mut cursor = Cursor { bytes, offset: 0 };
        if cursor.array::<8>()? != ACTOR_CARRIER_MAGIC || cursor.u32()? != ACTOR_CARRIER_VERSION {
            return Err(invalid("unsupported actor carrier"));
        }
        let texture_count = cursor.u32()? as usize;
        let binding_count = cursor.u32()? as usize;
        if cursor.u32()? != 0
            || texture_count > MAX_ACTOR_TEXTURES
            || binding_count > MAX_ACTOR_BINDINGS
        {
            return Err(invalid("actor carrier counts or padding are invalid"));
        }
        let manifest = cursor.array::<32>()?;
        let entity_identity = cursor.array::<32>()?;
        let policy = cursor.array::<32>()?;
        let payload_length = usize::try_from(cursor.u64()?)
            .map_err(|_| invalid("actor payload exceeds platform"))?;
        let end = HEADER
            .checked_add(payload_length)
            .filter(|end| end.checked_add(HASH) == Some(bytes.len()))
            .ok_or_else(|| invalid("actor carrier layout is invalid"))?;
        if Sha256::digest(&bytes[..end]).as_slice() != &bytes[end..]
            || policy != <[u8; 32]>::from(Sha256::digest(POLICY))
            || entity_identity != <[u8; 32]>::from(Sha256::digest(entity_bytes))
        {
            return Err(invalid("actor carrier identity mismatch"));
        }
        let entities = RuntimeEntityAssets::decode(entity_bytes)?;
        if manifest != entities.source_manifest_sha256() {
            return Err(invalid("actor entity manifest mismatch"));
        }
        cursor.bytes = &bytes[..end];
        let mut textures = Vec::with_capacity(texture_count);
        let mut total = 0usize;
        for _ in 0..texture_count {
            let source = cursor.u32()?;
            let width = cursor.u16()?;
            let height = cursor.u16()?;
            let pixel_sha256 = cursor.array()?;
            let length = pixel_length(width, height)?;
            total = total
                .checked_add(length)
                .filter(|total| *total <= MAX_ACTOR_PIXEL_BYTES)
                .ok_or_else(|| invalid("actor aggregate pixels exceed bounds"))?;
            textures.push(ActorTexture {
                source,
                width,
                height,
                pixel_sha256,
                rgba8: Arc::from(cursor.take(length)?),
            });
        }
        let mut bindings = Vec::with_capacity(binding_count);
        for _ in 0..binding_count {
            let rig = cursor.u32()?;
            let geometry_candidate = cursor.u32()?;
            let entity_symbol = cursor.u32()?;
            let geometry = cursor.u32()?;
            let render_controller = cursor.u32()?;
            let texture = cursor.u32()?;
            let pose_mode = match cursor.u32()? {
                0 => ActorPoseMode::CompiledLiteral,
                1 => ActorPoseMode::RestPose,
                _ => return Err(invalid("unknown actor pose mode")),
            };
            let length = cursor.u16()? as usize;
            if length == 0 || length > 128 {
                return Err(invalid("actor material name exceeds bound"));
            }
            let material = std::str::from_utf8(cursor.take(length)?)
                .map_err(|_| invalid("actor material is not UTF-8"))?
                .into();
            bindings.push(ActorArtworkBinding {
                rig,
                geometry_candidate,
                entity_symbol,
                geometry,
                render_controller,
                texture,
                material,
                pose_mode,
            });
        }
        if cursor.offset != end {
            return Err(invalid("actor carrier has trailing payload"));
        }
        validate(&textures, &bindings, &entities)?;
        Ok(Self {
            identity: Sha256::digest(bytes).into(),
            entity_identity,
            textures: textures.into(),
            bindings: bindings.into(),
        })
    }

    pub fn identity(&self) -> [u8; 32] {
        self.identity
    }
    pub fn entity_identity(&self) -> [u8; 32] {
        self.entity_identity
    }
    pub fn textures(&self) -> &[ActorTexture] {
        &self.textures
    }
    pub fn bindings(&self) -> &[ActorArtworkBinding] {
        &self.bindings
    }
    /// The first geometry candidate's binding of a rig.
    pub fn binding(&self, rig: u32) -> Option<&ActorArtworkBinding> {
        let index = self.bindings.partition_point(|binding| binding.rig < rig);
        self.bindings
            .get(index)
            .filter(|binding| binding.rig == rig)
    }
    /// The texture of the actor texture table drawn from entity-catalog source `source`.
    pub fn texture_of_source(&self, source: u32) -> Option<u32> {
        self.textures
            .iter()
            .position(|texture| texture.source == source)
            .map(|index| index as u32)
    }
}

pub fn encode_actor_catalog(
    entity_bytes: &[u8],
    textures: &[ActorTexture],
    bindings: &[ActorArtworkBinding],
) -> Result<Vec<u8>, AssetError> {
    let entities = RuntimeEntityAssets::decode(entity_bytes)?;
    validate(textures, bindings, &entities)?;
    let mut bytes = Vec::with_capacity(HEADER);
    bytes.extend_from_slice(&ACTOR_CARRIER_MAGIC);
    bytes.extend_from_slice(&ACTOR_CARRIER_VERSION.to_le_bytes());
    bytes.extend_from_slice(&(textures.len() as u32).to_le_bytes());
    bytes.extend_from_slice(&(bindings.len() as u32).to_le_bytes());
    bytes.extend_from_slice(&0u32.to_le_bytes());
    bytes.extend_from_slice(&entities.source_manifest_sha256());
    bytes.extend_from_slice(&Sha256::digest(entity_bytes));
    bytes.extend_from_slice(&Sha256::digest(POLICY));
    bytes.extend_from_slice(&0u64.to_le_bytes());
    for texture in textures {
        bytes.extend_from_slice(&texture.source.to_le_bytes());
        bytes.extend_from_slice(&texture.width.to_le_bytes());
        bytes.extend_from_slice(&texture.height.to_le_bytes());
        bytes.extend_from_slice(&texture.pixel_sha256);
        bytes.extend_from_slice(&texture.rgba8);
    }
    for binding in bindings {
        for value in [
            binding.rig,
            binding.geometry_candidate,
            binding.entity_symbol,
            binding.geometry,
            binding.render_controller,
            binding.texture,
            binding.pose_mode as u32,
        ] {
            bytes.extend_from_slice(&value.to_le_bytes());
        }
        bytes.extend_from_slice(&(binding.material.len() as u16).to_le_bytes());
        bytes.extend_from_slice(binding.material.as_bytes());
    }
    let length = bytes.len() - HEADER;
    bytes[120..128].copy_from_slice(&(length as u64).to_le_bytes());
    let hash = Sha256::digest(&bytes);
    bytes.extend_from_slice(&hash);
    if bytes.len() > MAX_ACTOR_CARRIER_BYTES {
        return Err(invalid("actor carrier exceeds bound"));
    }
    Ok(bytes)
}

fn validate(
    textures: &[ActorTexture],
    bindings: &[ActorArtworkBinding],
    entities: &RuntimeEntityAssets,
) -> Result<(), AssetError> {
    if textures.len() > MAX_ACTOR_TEXTURES || bindings.len() > MAX_ACTOR_BINDINGS {
        return Err(invalid("actor catalog counts exceed bounds"));
    }
    let mut total = 0usize;
    let mut seen_sources = std::collections::BTreeSet::new();
    for texture in textures {
        let length = pixel_length(texture.width, texture.height)?;
        total = total
            .checked_add(length)
            .filter(|total| *total <= MAX_ACTOR_PIXEL_BYTES)
            .ok_or_else(|| invalid("actor pixels exceed aggregate bound"))?;
        let source = entities
            .sources()
            .get(texture.source as usize)
            .ok_or_else(|| invalid("actor texture source is absent"))?;
        if !source.path.starts_with("textures/entity/")
            || !(source.path.ends_with(".png") || source.path.ends_with(".tga"))
            || texture.rgba8.len() != length
            || texture.pixel_sha256 != <[u8; 32]>::from(Sha256::digest(&texture.rgba8))
            || texture
                .rgba8
                .chunks_exact(4)
                .any(|pixel| !matches!(pixel[3], 0 | 255))
            || !seen_sources.insert(texture.source)
        {
            return Err(invalid("actor pixels or raster provenance are invalid"));
        }
    }
    let mut previous: Option<(u32, u32)> = None;
    for binding in bindings {
        let rig = entities
            .rig_bindings()
            .get(binding.rig as usize)
            .ok_or_else(|| invalid("actor rig is absent"))?;
        let candidates = rig.first_geometry..rig.first_geometry + u32::from(rig.geometry_count);
        let geometry_binding = entities
            .rig_geometries()
            .get(binding.geometry_candidate as usize)
            .ok_or_else(|| invalid("actor rig geometry is absent"))?;
        let geometry = entities
            .geometries()
            .get(binding.geometry as usize)
            .ok_or_else(|| invalid("actor geometry is absent"))?;
        let texture = textures
            .get(binding.texture as usize)
            .ok_or_else(|| invalid("actor texture index is absent"))?;
        let key = (binding.rig, binding.geometry_candidate);
        if previous.is_some_and(|previous| previous >= key)
            || binding.entity_symbol != rig.entity_symbol
            || !candidates.contains(&binding.geometry_candidate)
            || binding.render_controller != rig.render_controller
            || binding.geometry != geometry_binding.geometry
            || geometry.texture_width != texture.width
            || geometry.texture_height != texture.height
            || binding.material.is_empty()
            || !neutral_actor_geometry_uvs_are_supported(
                entities.geometries(),
                binding.geometry as usize,
            )
        {
            return Err(invalid("actor artwork binding is invalid or ambiguous"));
        }
        previous = Some(key);
    }
    Ok(())
}

fn pixel_length(width: u16, height: u16) -> Result<usize, AssetError> {
    if width == 0
        || height == 0
        || width > MAX_ACTOR_TEXTURE_SIDE
        || height > MAX_ACTOR_TEXTURE_SIDE
    {
        return Err(invalid("actor texture dimensions exceed bounds"));
    }
    usize::from(width)
        .checked_mul(usize::from(height))
        .and_then(|pixels| pixels.checked_mul(4))
        .ok_or_else(|| invalid("actor pixel length overflow"))
}

struct Cursor<'a> {
    bytes: &'a [u8],
    offset: usize,
}
impl<'a> Cursor<'a> {
    fn take(&mut self, length: usize) -> Result<&'a [u8], AssetError> {
        let end = self
            .offset
            .checked_add(length)
            .filter(|end| *end <= self.bytes.len())
            .ok_or_else(|| invalid("truncated actor carrier"))?;
        let value = &self.bytes[self.offset..end];
        self.offset = end;
        Ok(value)
    }
    fn array<const N: usize>(&mut self) -> Result<[u8; N], AssetError> {
        self.take(N)?
            .try_into()
            .map_err(|_| invalid("truncated actor field"))
    }
    fn u16(&mut self) -> Result<u16, AssetError> {
        Ok(u16::from_le_bytes(self.array()?))
    }
    fn u32(&mut self) -> Result<u32, AssetError> {
        Ok(u32::from_le_bytes(self.array()?))
    }
    fn u64(&mut self) -> Result<u64, AssetError> {
        Ok(u64::from_le_bytes(self.array()?))
    }
}
fn invalid(detail: &str) -> AssetError {
    AssetError::InvalidCompiledAssets {
        detail: detail.into(),
    }
}
