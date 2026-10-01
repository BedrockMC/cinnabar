//! Consent owns the whole input frame, including the frame that dismisses it.

use bevy::{
    input::{keyboard::KeyboardInput, mouse::MouseWheel},
    prelude::*,
};

#[derive(Resource, Default)]
pub(crate) struct ConsentInput(pub(crate) bool);

/// Removes queued physical events after consent reads them and before ordinary UI runs.
pub(crate) fn consume(world: &mut World) {
    if !world.resource::<ConsentInput>().0 {
        return;
    }
    world.resource_mut::<ButtonInput<KeyCode>>().reset_all();
    world.resource_mut::<ButtonInput<MouseButton>>().reset_all();
    if let Some(mut messages) = world.get_resource_mut::<Messages<KeyboardInput>>() {
        messages.clear();
    }
    if let Some(mut messages) = world.get_resource_mut::<Messages<MouseWheel>>() {
        messages.clear();
    }
}

/// Keeps ordinary UI readers from observing controller and touch input owned by consent.
pub(crate) fn ordinary_input(consent: Option<Res<ConsentInput>>) -> bool {
    !consent.is_some_and(|consent| consent.0)
}
