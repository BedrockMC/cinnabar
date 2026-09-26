//! Real ordered-stream commit-to-authority witnesses, not network-ingress tests.
use super::*;
use crate::{
    acceptance::{AcceptanceRun, model_witness::ModelWitnessFileSource},
    app::{
        ClientBlobCacheOwner, configure_client_authority_systems, configure_client_frame_schedule,
    },
    camera::CameraSettingsAuthority,
    environment::{WeatherState, bind_session_generation},
    local_player::{InteractionOriginSnapshot, LocalPlayerFrameCarrier, LocalViewPose},
    menu::{CoreProcessGuard, MenuClipboard, MenuRuntime},
    movement::{
        LocalMovementEffectTimeline, LocalMovementSpeedAuthority, LocalPhysicsController,
        MovementTicker, PhysicsCollisionRegistries,
    },
    runtime::{
        network::{NetworkHandle, ResourcePackAdmissionState},
        phase3_evidence::Phase3EvidenceEmitter,
    },
    semantic_controls::{
        PendingDeviceFrame, SemanticInputRuntime, SemanticInputSnapshot, SemanticRouteState,
        SemanticTouchTargets,
    },
    server_camera::ServerCameraInstructions,
    settings_runtime::RuntimeSettings,
    ui_runtime::{
        LocalFormAction, flush_form_response,
        presentation::{UiPresentationRuntime, tests::fixture_font},
    },
};
use bevy::{
    input::{
        ButtonState,
        keyboard::{Key, KeyboardInput, NativeKey},
        mouse::AccumulatedMouseMotion,
    },
    prelude::*,
    window::{CursorGrabMode, CursorOptions, PrimaryWindow},
};
use protocol::{
    FormKind, FormRequestEvent, ServerFormModel, TextMenuForm, UiEvent, WorldBootstrap, WorldEvent,
};
use render::ChunkUploadBudget;
use semantic_input::Action;
use std::sync::Arc;

#[derive(Clone, Copy, Debug)]
enum InputCase {
    Move,
    Attack,
    Use,
    Chat,
    Inventory,
    Pause,
}

fn fixture_app() -> (App, Entity) {
    let mut clock = WorldClock::default();
    let mut weather = WeatherState::default();
    bind_session_generation(&mut clock, &mut weather, 1);
    let stream = client_world::WorldStream::new(WorldBootstrap {
        dimension: 0,
        local_player_runtime_id: 42,
        local_player_unique_id: 1,
        player_position: [0.0, 70.0, 0.0],
        world_spawn_position: [0, 70, 0],
        air_network_id: protocol::SEQUENTIAL_AIR_NETWORK_ID,
        block_network_ids_are_hashes: false,
    });
    let breg = include_bytes!("../../../../../crates/assets/data/block-registry-v2168.bin");
    let preg = include_bytes!("../../../../../crates/assets/data/block-physics-v2168.bin");
    let records = assets::read_registry_for_protocol(breg, 2168).unwrap();
    let collisions = PhysicsCollisionRegistries::from_assets(breg, &records, preg, 2168).unwrap();
    let mut menu = MenuRuntime::new(false, 2, "Test".into());
    menu.set_visible(false);
    let mut app = App::new();
    let mut runtime = UiRuntime::new(1);
    runtime.publish_inventory_authority(protocol::InventoryAuthority::Server);
    runtime.publish_local_runtime_id(1, 42).unwrap();
    configure_client_frame_schedule(&mut app);
    configure_client_authority_systems(&mut app);
    app.add_message::<KeyboardInput>()
        .add_message::<AppExit>()
        .insert_resource(ClientWorld {
            stream: Some(stream),
            ..ClientWorld::default()
        })
        .insert_resource(clock)
        .insert_resource(weather)
        .insert_resource(collisions)
        .insert_resource(runtime)
        .insert_resource(UiPresentationRuntime::new(fixture_font()).unwrap())
        .insert_resource(menu)
        .insert_resource(NetworkHandle::disconnected())
        .insert_resource(AcceptanceRun::new(Some(900), None, false, false))
        .insert_resource(ModelWitnessFileSource::new(None))
        .init_resource::<MovementTicker>()
        .init_resource::<LocalPhysicsController>()
        .init_resource::<LocalMovementEffectTimeline>()
        .init_resource::<LocalMovementSpeedAuthority>()
        .init_resource::<Time<Real>>()
        .init_resource::<ChunkUploadBudget>()
        .init_resource::<CameraSettingsAuthority>()
        .init_resource::<LocalViewPose>()
        .init_resource::<LocalPlayerFrameCarrier>()
        .init_resource::<InteractionOriginSnapshot>()
        .init_resource::<Phase3EvidenceEmitter>()
        .init_resource::<ServerCameraInstructions>()
        .init_resource::<CoreProcessGuard>()
        .init_resource::<ClientBlobCacheOwner>()
        .init_resource::<ResourcePackAdmissionState>()
        .init_resource::<MenuClipboard>()
        .init_resource::<ButtonInput<KeyCode>>()
        .init_resource::<ButtonInput<MouseButton>>()
        .init_resource::<AccumulatedMouseMotion>()
        .init_resource::<Touches>()
        .init_resource::<SemanticInputRuntime>()
        .init_resource::<SemanticInputSnapshot>()
        .init_resource::<PendingDeviceFrame>()
        .init_resource::<SemanticRouteState>()
        .init_resource::<SemanticTouchTargets>()
        .init_resource::<RuntimeSettings>();
    let window = app
        .world_mut()
        .spawn((
            Window {
                focused: true,
                ..Default::default()
            },
            CursorOptions {
                visible: false,
                grab_mode: CursorGrabMode::Locked,
                ..Default::default()
            },
            PrimaryWindow,
        ))
        .id();
    app.update(); // Bind the real semantic authority before any test edge.
    (app, window)
}

fn submit_form(app: &mut App, sequence: u64) {
    app.world_mut()
        .resource_mut::<ClientWorld>()
        .stream
        .as_mut()
        .unwrap()
        .submit(
            sequence,
            WorldEvent::Ui(UiEvent::Form(FormRequestEvent {
                form_id: 7,
                kind: FormKind::Menu,
                title: Some(Arc::from("Choose 世界")),
                json: Arc::from("{}"),
                model: ServerFormModel::TextMenu(TextMenuForm {
                    title: Arc::from("Choose 世界"),
                    content: Arc::from("Pick one"),
                    buttons: vec![Arc::from("First ✓"), Arc::from("第二")].into(),
                    omitted_images: 0,
                }),
            })),
        )
        .unwrap();
}

fn press(app: &mut App, window: Entity, case: InputCase) {
    let key = match case {
        InputCase::Move => KeyCode::KeyW,
        InputCase::Chat => KeyCode::KeyT,
        InputCase::Inventory => KeyCode::KeyE,
        InputCase::Pause => KeyCode::Escape,
        InputCase::Attack | InputCase::Use => {
            app.world_mut()
                .resource_mut::<ButtonInput<MouseButton>>()
                .press(if matches!(case, InputCase::Attack) {
                    MouseButton::Left
                } else {
                    MouseButton::Right
                });
            return;
        }
    };
    app.world_mut()
        .resource_mut::<ButtonInput<KeyCode>>()
        .press(key);
    app.world_mut().write_message(KeyboardInput {
        key_code: key,
        logical_key: Key::Unidentified(NativeKey::Unidentified),
        state: ButtonState::Pressed,
        text: None,
        repeat: false,
        window,
    });
}

fn assert_positive_control(app: &App, case: InputCase) {
    let ui = app.world().resource::<UiRuntime>();
    let input = app.world().resource::<SemanticInputSnapshot>();
    match case {
        InputCase::Move => assert_ne!(
            input.movement(),
            [0.0; 2],
            "movement edge must normally reach semantic consumers"
        ),
        InputCase::Attack => assert!(
            input.phase(Action::Attack).pressed,
            "attack positive control"
        ),
        InputCase::Use => assert!(input.phase(Action::Use).pressed, "use positive control"),
        InputCase::Chat => assert!(ui.chat_focused(), "chat positive control"),
        InputCase::Inventory => assert!(ui.inventory_open(), "inventory positive control"),
        InputCase::Pause => assert!(
            app.world().resource::<MenuRuntime>().is_visible(),
            "pause positive control"
        ),
    }
}

#[test]
fn committed_form_owns_first_visible_frame_and_recovers_each_real_input_consumer() {
    for case in [
        InputCase::Move,
        InputCase::Attack,
        InputCase::Use,
        InputCase::Chat,
        InputCase::Inventory,
        InputCase::Pause,
    ] {
        let (mut control, control_window) = fixture_app();
        press(&mut control, control_window, case);
        control.update();
        assert_positive_control(&control, case);

        let (mut app, window) = fixture_app();
        submit_form(&mut app, 1); // Real ordered stream; never pre-admit UiRuntime.
        press(&mut app, window, case);
        app.update();
        let runtime = app.world().resource::<UiRuntime>();
        assert!(
            runtime.server_forms().owns_input(),
            "{case:?}: first-visible-frame authority"
        );
        assert!(!runtime.chat_focused() && !runtime.inventory_open());
        assert!(!app.world().resource::<MenuRuntime>().is_visible());
        let input = app.world().resource::<SemanticInputSnapshot>();
        assert_eq!(
            input.movement(),
            [0.0; 2],
            "{case:?}: admission-frame gameplay movement"
        );
        assert_eq!(input.phase(Action::Attack), Default::default());
        assert_eq!(input.phase(Action::Use), Default::default());
        let cursor = app.world().get::<CursorOptions>(window).unwrap();
        assert!(
            cursor.visible,
            "{case:?}: first-visible-frame cursor is released"
        );
        assert_eq!(cursor.grab_mode, CursorGrabMode::None);
        assert!(
            app.world().resource::<ClientWorld>().fatal_error.is_none(),
            "real stream drain must succeed"
        );
        let mut runtime = app.world_mut().remove_resource::<UiRuntime>().unwrap();
        app.world_mut()
            .resource_mut::<UiPresentationRuntime>()
            .build(&runtime, 0, [1280, 720], ui::DpiScale::new(1.0).unwrap())
            .unwrap();
        if let Some(form) = runtime.server_forms().active() {
            let identity = form.identity;
            assert_eq!(
                app.world()
                    .resource::<UiPresentationRuntime>()
                    .form_button_count(identity),
                Some(2)
            );
            runtime
                .respond_to_server_form(identity, LocalFormAction::Dismiss)
                .unwrap();
        }
        assert!(flush_form_response(&mut runtime, |_| Ok(())).unwrap());
        assert!(!flush_form_response(&mut runtime, |_| Ok(())).unwrap());
        app.insert_resource(runtime);
        app.update(); // Restore cursor/input without replaying the old edge.
        let cursor = app.world().get::<CursorOptions>(window).unwrap();
        assert!(!cursor.visible && cursor.grab_mode == CursorGrabMode::Locked);
        assert!(!app.world().resource::<UiRuntime>().ui_focused());
        press(&mut app, window, case);
        app.update();
        assert_positive_control(&app, case);
    }
}
