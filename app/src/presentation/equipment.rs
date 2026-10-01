//! Worn armor and held items as extra rig layers that ride the owning actor's pose.

mod armor;
mod atlas;
mod attachable;
mod blocks;
mod display;
mod elytra;
#[cfg(test)]
mod frames;
mod input;
mod runtime;
#[cfg(test)]
mod tests;

pub(crate) use display::FirstPersonHand;
pub(crate) use input::{local_input, remote_input};
#[cfg(test)]
pub(crate) use runtime::{ActorEquipmentInput, HeldKind, WornItem};
pub(crate) use runtime::{
    EquipmentPresentation, EquipmentRuntime, FirstPersonArms, FirstPersonItem, StagedSessionIcons,
};
