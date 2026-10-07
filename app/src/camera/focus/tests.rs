use super::*;
use client_ui::ui_runtime::UiRuntime;

/// Builds the production focus/capture boundary without a windowing plugin or OS input.
fn focus_app() -> (App, Entity) {
    let mut app = App::new();
    app.init_resource::<ButtonInput<KeyCode>>()
        .init_resource::<ButtonInput<MouseButton>>()
        .init_resource::<AccumulatedMouseMotion>()
        .insert_resource(AutoFly::new(false))
        .insert_resource(crate::player_runtime::PlayerRuntime::new(1))
        .insert_resource(UiRuntime::new(1))
        .add_systems(Update, update_cursor_capture);
    install(&mut app);
    let window = app
        .world_mut()
        .spawn((
            Window::default(),
            CursorOptions {
                grab_mode: CursorGrabMode::Locked,
                visible: false,
                ..default()
            },
            PrimaryWindow,
        ))
        .id();
    (app, window)
}

/// Checks both the OS request and the gameplay capture observation.
fn assert_released(app: &App, window: Entity) {
    let cursor = app.world().get::<CursorOptions>(window).unwrap();
    assert_eq!(cursor.grab_mode, CursorGrabMode::None);
    assert!(cursor.visible);
    assert!(!super::super::input_is_active(
        app.world().get::<Window>(window).unwrap(),
        cursor
    ));
}

#[test]
fn transient_overlay_focus_loss_beats_hud_capture_and_same_frame_click() {
    let (mut app, window) = focus_app();
    app.update();
    app.world_mut().write_message(WindowFocused {
        window,
        focused: false,
    });
    app.world_mut().write_message(WindowFocused {
        window,
        focused: true,
    });
    app.world_mut()
        .resource_mut::<ButtonInput<MouseButton>>()
        .press(MouseButton::Left);
    app.world_mut()
        .resource_mut::<ButtonInput<KeyCode>>()
        .press(KeyCode::KeyW);
    app.world_mut()
        .resource_mut::<AccumulatedMouseMotion>()
        .delta = Vec2::new(20.0, 10.0);
    app.update();
    assert_released(&app, window);
    assert_eq!(
        app.world().resource::<AccumulatedMouseMotion>().delta,
        Vec2::ZERO
    );
    assert!(
        !app.world()
            .resource::<ButtonInput<KeyCode>>()
            .pressed(KeyCode::KeyW)
    );
    app.update();
    assert_released(&app, window);
    app.world_mut()
        .resource_mut::<ButtonInput<MouseButton>>()
        .press(MouseButton::Left);
    app.update();
    let cursor = app.world().get::<CursorOptions>(window).unwrap();
    assert_eq!(cursor.grab_mode, CursorGrabMode::Locked);
    assert!(!cursor.visible);
    assert!(
        !app.world()
            .resource::<ButtonInput<MouseButton>>()
            .pressed(MouseButton::Left)
    );
}

#[test]
fn occlusion_releases_until_unoccluded_and_clicked_and_ignores_other_windows() {
    let (mut app, window) = focus_app();
    let secondary = app.world_mut().spawn(Window::default()).id();
    app.world_mut().write_message(WindowFocused {
        window: secondary,
        focused: false,
    });
    app.world_mut().write_message(WindowOccluded {
        window: secondary,
        occluded: true,
    });
    app.update();
    assert_eq!(
        app.world().get::<CursorOptions>(window).unwrap().grab_mode,
        CursorGrabMode::Locked
    );
    app.world_mut().write_message(WindowOccluded {
        window,
        occluded: true,
    });
    app.update();
    assert_released(&app, window);
    app.world_mut().write_message(WindowFocused {
        window,
        focused: true,
    });
    app.update();
    assert_released(&app, window);
    app.world_mut().write_message(WindowOccluded {
        window,
        occluded: false,
    });
    app.update();
    assert_released(&app, window);
    app.world_mut()
        .resource_mut::<ButtonInput<MouseButton>>()
        .press(MouseButton::Left);
    app.update();
    assert_eq!(
        app.world().get::<CursorOptions>(window).unwrap().grab_mode,
        CursorGrabMode::Locked
    );
}

/// Simulates a screen adapter restoring a previously captured cursor.
fn restore_screen_capture(mut cursors: Query<&mut CursorOptions, With<PrimaryWindow>>) {
    for mut cursor in &mut cursors {
        cursor.grab_mode = CursorGrabMode::Locked;
        cursor.visible = false;
    }
}

#[test]
fn late_screen_capture_cannot_override_loss_or_driven_control() {
    let (mut app, window) = focus_app();
    app.add_systems(Update, restore_screen_capture.after(update_cursor_capture));
    app.world_mut().get_mut::<Window>(window).unwrap().focused = false;
    app.update();
    assert_released(&app, window);
    app.world_mut().get_mut::<Window>(window).unwrap().focused = true;
    app.update();
    assert_released(&app, window);
    app.init_resource::<DrivenInput>();
    app.update();
    assert_released(&app, window);
}

#[test]
fn json_ui_form_release_restores_capture_after_overlay_and_response_delivery() {
    use crate::ui_runtime::{
        drive_server_form_input,
        presentation::forms::{pack_harness, tests::mini_engine_presentation},
    };
    use bevy::input::{ButtonState, InputPlugin, mouse::MouseButtonInput};
    use client_ui::ui_runtime::flush_form_response;

    let mut player = crate::player_runtime::PlayerRuntime::new(1);
    let runtime = pack_harness::action_form(&mut player, "Menu", &["A", "B"]);
    let mut presentation = mini_engine_presentation();
    let (physical, dpi) = ([1280, 720], ui::DpiScale::new(1.0).unwrap());
    presentation
        .build(&player, &runtime, 0, physical, dpi)
        .unwrap();
    let identity = runtime.server_forms().active().unwrap().identity;
    let frame = presentation.form_engine_frame(identity).unwrap();
    let hit = frame
        .hits
        .iter()
        .find(|hit| hit.collection_index == Some(1))
        .unwrap();
    let centre = Vec2::new(
        frame.origin[0] + (hit.rect.x + hit.rect.w / 2.0) as f32 * frame.scale,
        frame.origin[1] + (hit.rect.y + hit.rect.h / 2.0) as f32 * frame.scale,
    );
    let mut app = App::new();
    app.add_plugins(InputPlugin)
        .insert_resource(AutoFly::new(false))
        .insert_resource(player)
        .insert_resource(runtime)
        .insert_resource(presentation)
        .add_systems(
            Update,
            (drive_server_form_input, update_cursor_capture).chain(),
        );
    install(&mut app);
    let mut window = Window {
        focused: false,
        ..default()
    };
    window
        .resolution
        .set_physical_resolution(physical[0], physical[1]);
    window.set_cursor_position(Some(centre));
    let window = app
        .world_mut()
        .spawn((window, CursorOptions::default(), PrimaryWindow))
        .id();
    app.update();
    app.world_mut().get_mut::<Window>(window).unwrap().focused = true;
    app.update();
    assert_released(&app, window);
    app.world_mut().write_message(MouseButtonInput {
        button: MouseButton::Left,
        state: ButtonState::Pressed,
        window,
    });
    app.update();
    assert!(
        app.world()
            .resource::<UiRuntime>()
            .server_forms()
            .active()
            .is_some()
    );
    app.world_mut().write_message(MouseButtonInput {
        button: MouseButton::Left,
        state: ButtonState::Released,
        window,
    });
    app.update();
    assert!(
        app.world()
            .resource::<UiRuntime>()
            .server_forms()
            .active()
            .is_none()
    );
    assert!(
        app.world()
            .resource::<UiRuntime>()
            .server_forms()
            .owns_input()
    );
    assert_released(&app, window);
    flush_form_response(&mut app.world_mut().resource_mut::<UiRuntime>(), |_| Ok(())).unwrap();
    app.update();
    let cursor = app.world().get::<CursorOptions>(window).unwrap();
    assert_eq!(cursor.grab_mode, CursorGrabMode::Locked);
    assert!(!cursor.visible);
}
