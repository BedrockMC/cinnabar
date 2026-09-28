//! Worn armor and held items as extra rig layers that ride the owning actor's pose.

mod armor;
mod atlas;
mod display;
mod runtime;
#[cfg(test)]
mod tests;

pub(crate) use runtime::{ActorEquipmentInput, EquipmentPresentation, EquipmentRuntime, WornItem};
