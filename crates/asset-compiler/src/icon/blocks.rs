//! Block facts icon baking needs beyond the world carrier: registry families, the pack's carried
//! textures, and the canonical state an item is drawn from.
//!
//! Vanilla's item renderer draws a block item flat when `BlockTessellator::canRender` rejects
//! its shape; `BlockItem::getIconInfo` then shows the carried texture, down face, at the
//! block's variant.

use std::path::Path;

use assets::{
    AssetError, BlockFace, BlockVisualId, IconSprite, ModelFamily, NetworkIdMode, RegistryRecord,
    RuntimeAssets, VisualKind, read_registry_for_protocol,
};

use crate::compiler::static_texture_path;
use crate::image::decode_texture;
use crate::pack::{
    PackSources, read_pack, resolve_carried_down_key, resolve_carried_face_key, resolve_texture_key,
};

/// Registry families whose vanilla block shapes render as flat item icons.
const FLAT_FAMILIES: [ModelFamily; 11] = [
    ModelFamily::Cross,
    ModelFamily::Rail,
    ModelFamily::Torch,
    ModelFamily::Lever,
    ModelFamily::Vine,
    ModelFamily::GlowLichen,
    ModelFamily::SculkVein,
    ModelFamily::Pane,
    ModelFamily::Aquatic,
    ModelFamily::FlowerBed,
    ModelFamily::ResinClump,
];

/// Registry-unclassified blocks whose vanilla items show a flat icon (lanterns, candles,
/// chains, ladders, lily pads, end rods, sea pickles, spore blossoms, bamboo).
fn is_reviewed_flat(name: &str) -> bool {
    let name = name.strip_prefix("minecraft:").unwrap_or(name);
    ["lantern", "candle", "chain"]
        .iter()
        .any(|suffix| name.ends_with(suffix))
        || matches!(
            name,
            "ladder" | "waterlily" | "end_rod" | "sea_pickle" | "spore_blossom" | "bamboo"
        )
}

pub(super) struct IconBlocks {
    records: Box<[RegistryRecord]>,
    /// `None` when the pack lacks the block and terrain catalogs.
    pack: Option<PackSources>,
}

impl IconBlocks {
    pub(super) fn read(root: &Path) -> Result<Self, AssetError> {
        let records = read_registry_for_protocol(
            include_bytes!("../../../assets/data/block-registry-v2193.bin"),
            2193,
        )?;
        let has_catalogs = root.join("blocks.json").is_file()
            && root.join("textures/terrain_texture.json").is_file();
        Ok(Self {
            records,
            pack: has_catalogs.then(|| read_pack(root)).transpose()?,
        })
    }

    pub(super) fn is_flat(&self, world: &RuntimeAssets, visual: BlockVisualId) -> bool {
        let Some(record) = self.records.get(visual.0 as usize) else {
            return false;
        };
        let kind = world.resolve(NetworkIdMode::Sequential, visual.0).kind();
        FLAT_FAMILIES.contains(&record.model_family)
            || (kind == VisualKind::Cross && record.model_family != ModelFamily::Crop)
            || (record.model_family == ModelFamily::Unknown
                && kind == VisualKind::Model
                && is_reviewed_flat(&record.name))
    }

    /// The state an item draws: a wall item shows a post with east and west arms.
    pub(super) fn icon_state(&self, visual: BlockVisualId) -> BlockVisualId {
        let Some(record) = self.records.get(visual.0 as usize) else {
            return visual;
        };
        if record.model_family != ModelFamily::Wall {
            return visual;
        }
        let wanted = |state: &str| {
            let Ok(state) = serde_json::from_str::<serde_json::Value>(state) else {
                return false;
            };
            let value = |key: &str| state.get(key).and_then(|entry| entry.get("value")).cloned();
            value("wall_post_bit") == Some(1.into())
                && ["east", "west"].iter().all(|side| {
                    value(&format!("wall_connection_type_{side}")) == Some("short".into())
                })
                && ["north", "south"].iter().all(|side| {
                    value(&format!("wall_connection_type_{side}")) == Some("none".into())
                })
        };
        self.records
            .iter()
            .find(|candidate| candidate.name == record.name && wanted(&candidate.canonical_state))
            .map_or(visual, |candidate| BlockVisualId(candidate.sequential_id))
    }

    /// Carried face paths in `BlockFace` order for a block whose icon uses them (leaves).
    pub(super) fn carried_faces(&self, visual: BlockVisualId) -> Option<[Box<str>; 6]> {
        let pack = self.pack.as_ref()?;
        let record = self.records.get(visual.0 as usize)?;
        let mut paths = BlockFace::ALL.map(|_| Box::<str>::default());
        for face in BlockFace::ALL {
            let key = resolve_carried_face_key(&pack.blocks, record, face)?;
            paths[face as usize] = pack.terrain.get_clamped_untinted(&key, 0)?.into();
        }
        Some(paths)
    }

    /// The pack path of `visual`'s flat icon; `None` when absent, tinted, or unresolvable.
    pub(super) fn texture_path(&self, visual: BlockVisualId) -> Option<Box<str>> {
        let pack = self.pack.as_ref()?;
        let record = self.records.get(visual.0 as usize)?;
        let key = resolve_carried_down_key(&pack.blocks, record)?;
        // The world key's variant stands in for the block's `getVariant`.
        let variant = resolve_texture_key(&pack.blocks, record, BlockFace::Down)
            .key
            .and_then(|world| pack.terrain.get_for_model_record(&world, record))
            .map_or(0, |(_, variant)| variant as usize);
        pack.terrain
            .get_clamped_untinted(&key, variant)
            .map(Into::into)
    }

    pub(super) fn sprite(root: &Path, path: &str) -> Result<Option<IconSprite>, AssetError> {
        let file = static_texture_path(root, path, path)?;
        if !file.is_file() {
            return Ok(None);
        }
        Ok(super::bounded_sprite(decode_texture(&file, path)?).map(|(sprite, _)| sprite))
    }

    /// A 16x16 terrain tile (a strip's first frame); `None` for any other size.
    pub(super) fn tile(root: &Path, path: &str) -> Result<Option<Box<[u8]>>, AssetError> {
        Ok(Self::sprite(root, path)?
            .filter(|sprite| sprite.width == 16 && sprite.height == 16)
            .map(|sprite| sprite.rgba8.to_vec().into_boxed_slice()))
    }
}
