//! Player skins' own models registered as actor rig geometry, shared by model digest.
use assets::SkinGeometry;
use render::{ActorRigGeometry, EntityRigId, skin_geometry, skin_rig_id};

/// Distinct skin models kept registered; the least recently drawn one is replaced when full.
pub(crate) const MAX_SKIN_RIGS: usize = 64;
/// Models that failed to build, remembered so they are not rebuilt every frame.
const MAX_REJECTED_SKIN_RIGS: usize = 64;

struct Slot {
    digest: [u8; 32],
    last_used: u64,
}

#[derive(Default)]
pub(crate) struct SkinRigCache {
    slots: Vec<Slot>,
    rejected: Vec<[u8; 32]>,
    frame: u64,
}

impl SkinRigCache {
    pub(crate) fn begin_frame(&mut self) {
        self.frame = self.frame.wrapping_add(1);
    }

    /// The rig drawing `geometry`, built and handed to `register` on first use; `None` when the
    /// model cannot be built (see [`Self::rejects`]) or every slot already draws this frame.
    pub(crate) fn rig(
        &mut self,
        geometry: &SkinGeometry,
        mut register: impl FnMut(ActorRigGeometry),
    ) -> Option<EntityRigId> {
        if let Some(index) = self
            .slots
            .iter()
            .position(|slot| slot.digest == geometry.digest)
        {
            self.slots[index].last_used = self.frame;
            return Some(skin_rig_id(index as u32));
        }
        if self.rejected.contains(&geometry.digest) {
            return None;
        }
        let index = if self.slots.len() < MAX_SKIN_RIGS {
            self.slots.len()
        } else {
            // Evicting a slot drawn this frame would swap another player's model and rebuild
            // the geometry catalog every frame.
            self.slots
                .iter()
                .enumerate()
                .filter(|(_, slot)| slot.last_used != self.frame)
                .min_by_key(|(_, slot)| slot.last_used)
                .map(|(index, _)| index)?
        };
        let id = skin_rig_id(index as u32);
        let Ok(built) = skin_geometry(geometry, id) else {
            if self.rejected.len() == MAX_REJECTED_SKIN_RIGS {
                self.rejected.remove(0);
            }
            self.rejected.push(geometry.digest);
            bevy::log::warn!(identifier = %geometry.identifier, "skin model could not be built");
            return None;
        };
        register(built);
        let slot = Slot {
            digest: geometry.digest,
            last_used: self.frame,
        };
        if index == self.slots.len() {
            self.slots.push(slot);
        } else {
            self.slots[index] = slot;
        }
        Some(id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PATCH: &str = r#"{"geometry":{"default":"geometry.test"}}"#;
    const MODEL: &str = r#"{"format_version":"1.12.0","minecraft:geometry":[{
 "description":{"identifier":"geometry.test","texture_width":64,"texture_height":64},
 "bones":[{"name":"body","pivot":[0,24,0],"cubes":[{"origin":[-4,12,-2],"size":[8,12,4],"uv":[16,16]}]}]}]}"#;

    /// More distinct models than slots in one frame must not evict models drawn this frame.
    #[test]
    fn a_crowded_frame_never_evicts_its_own_models() {
        let mut model = assets::parse_skin_geometry(PATCH, MODEL).unwrap().unwrap();
        let models: Vec<SkinGeometry> = (0..=MAX_SKIN_RIGS)
            .map(|index| {
                model.digest[..8].copy_from_slice(&(index as u64).to_le_bytes());
                model.clone()
            })
            .collect();
        let mut cache = SkinRigCache::default();
        let mut builds = 0;
        for frame in 0..3 {
            cache.begin_frame();
            let ids: Vec<_> = models
                .iter()
                .map(|model| cache.rig(model, |_| builds += 1))
                .collect();
            assert_eq!(ids.iter().flatten().count(), MAX_SKIN_RIGS, "frame {frame}");
            assert!(ids[MAX_SKIN_RIGS].is_none());
        }
        assert_eq!(builds, MAX_SKIN_RIGS, "each model built once");
    }
}
