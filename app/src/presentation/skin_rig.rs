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
    /// model cannot be built as rig geometry.
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
            self.slots
                .iter()
                .enumerate()
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
