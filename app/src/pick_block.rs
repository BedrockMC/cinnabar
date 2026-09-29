//! Middle-click pick block: asks the server to put the targeted block in hand.

use bevy::{
    ecs::system::SystemParam,
    prelude::{ButtonInput, MouseButton, Query, Res, ResMut, Window, With},
    window::PrimaryWindow,
};
use protocol::PlayerGameMode;

use crate::{
    interaction_authority::observe_block,
    local_player::InteractionOriginSnapshot,
    menu::MenuRuntime,
    mining::{creative_reach, hand_interaction_selection, protocol_input_mode, survival_reach},
    movement::PhysicsCollisionRegistries,
    runtime::{network::NetworkHandle, world::ClientWorld},
    semantic_controls::SemanticInputSnapshot,
    ui_runtime::UiRuntime,
};

#[derive(SystemParam)]
pub(crate) struct PickBlockContext<'w, 's> {
    input: Res<'w, SemanticInputSnapshot>,
    origin: Res<'w, InteractionOriginSnapshot>,
    menu: Res<'w, MenuRuntime>,
    windows: Query<'w, 's, &'static Window, With<PrimaryWindow>>,
    client_world: Res<'w, ClientWorld>,
    collisions: Res<'w, PhysicsCollisionRegistries>,
    mouse: Res<'w, ButtonInput<MouseButton>>,
    network: Res<'w, NetworkHandle>,
}

/// Sends one block-pick request per middle-click on a block in reach.
pub(crate) fn produce_pick_block(context: PickBlockContext, ui: ResMut<UiRuntime>) {
    if !context.mouse.just_pressed(MouseButton::Middle)
        || context.menu.is_visible()
        || ui.ui_focused()
        || !context.windows.single().is_ok_and(|window| window.focused)
        || ui
            .player_game_mode()
            .is_some_and(|mode| !mode.shows_hotbar())
    {
        return;
    }
    let (Some(snapshot), Some(selection)) =
        (context.input.snapshot(), hand_interaction_selection(&ui))
    else {
        return;
    };
    let input_mode = protocol_input_mode(snapshot.input_mode);
    let reach = if ui.player_game_mode() == Some(PlayerGameMode::Creative) {
        creative_reach(input_mode)
    } else {
        survival_reach(input_mode)
    };
    let Some(observed) = observe_block(
        &context.origin,
        &ui,
        &context.client_world,
        &context.collisions,
        selection,
        (
            input_mode,
            reach,
            (snapshot.authority_generation, snapshot.frame_sequence),
            0,
        ),
    ) else {
        return;
    };
    // A full queue drops the pick; the player simply clicks again.
    let _ = context
        .network
        .send_inventory_packet(protocol::block_pick_request_packet(
            observed.target.position,
            false,
        ));
}
