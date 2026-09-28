//! Worn armor and held items as extra rig layers that ride the owning actor's pose.

mod armor;
mod atlas;
mod blocks;
mod display;
mod elytra;
mod input;
mod runtime;
#[cfg(test)]
mod tests;

pub(crate) use input::{local_input, remote_input};
pub(crate) use runtime::{
    ActorEquipmentInput, EquipmentPresentation, EquipmentRuntime, FirstPersonArms, HeldKind,
    WornItem,
};
