//! Shared production commit-to-input authority registrations.
use super::*;
use crate::runtime::world::drain_committed_ui_before_authority;

pub(crate) fn configure_client_frame_schedule(app: &mut App) {
    app.configure_sets(
        Update,
        (
            ClientFrameSet::RawInput,
            ClientFrameSet::SemanticSample,
            ClientFrameSet::UiAuthority,
            ClientFrameSet::SemanticFinalize,
            ClientFrameSet::Physics,
            ClientFrameSet::Camera,
            ClientFrameSet::Interaction,
            ClientFrameSet::WorldPublication,
            ClientFrameSet::ActorPublication,
            ClientFrameSet::UiPublication,
            ClientFrameSet::NetworkSend,
        )
            .chain(),
    );
}

pub(crate) fn configure_client_authority_systems(app: &mut App) {
    app.add_message::<crate::runtime::audio::SequencedAudioEvent>()
        .add_message::<bevy::input::mouse::MouseWheel>()
        .init_resource::<WorldStreamFramePoll>()
        .add_systems(
            Update,
            (drive_gameplay_touch_targets, collect_raw_input)
                .chain()
                .in_set(ClientFrameSet::RawInput),
        )
        .add_systems(
            Update,
            route_semantic_input.in_set(ClientFrameSet::SemanticSample),
        )
        .add_systems(
            Update,
            (
                drive_sign_editor,
                drive_server_form_input,
                drive_chat_ui_actions,
                drain_inventory_authority,
                drive_chat_keyboard_input,
                drive_menu_input,
                drive_inventory_ui_actions,
                drive_menu_connection,
                synchronize_semantic_input_authority,
                drive_world_inventory_keys,
            )
                .chain()
                .in_set(ClientFrameSet::UiAuthority),
        )
        .add_systems(
            Update,
            finalize_semantic_input_after_ui_authority.in_set(ClientFrameSet::SemanticFinalize),
        )
        .add_systems(
            Update,
            reconcile_world_stream_before_physics
                .after(receive_network_events)
                .before(drain_committed_ui_before_authority)
                .before(ClientFrameSet::UiAuthority)
                .before(ClientFrameSet::Physics),
        )
        .add_systems(
            Update,
            drain_committed_ui_before_authority
                .after(reconcile_world_stream_before_physics)
                .before(ClientFrameSet::UiAuthority),
        );
}
