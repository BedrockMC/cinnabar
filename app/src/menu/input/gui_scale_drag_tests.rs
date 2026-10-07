use bevy::{prelude::*, window::WindowResolution};
use ui::{DpiScale, UiRect};

use super::*;
use crate::{
    menu::{MenuAction, MenuScreen},
    ui_runtime::presentation::{
        apply_gui_scale_setting, tests::engine_hud_tests::engine_presentation,
    },
};
use client_ui::ui_runtime::UiRuntime;

const PHYSICAL: [u32; 2] = [1920, 1080];

fn frame(app: &mut App, action: MenuAction) -> UiRect {
    let view = app.world().resource::<MenuRuntime>().view();
    app.world_mut()
        .resource_scope(|world, mut presentation: Mut<UiPresentationRuntime>| {
            presentation.set_menu_view(Some(view));
            let pane = UiPoint::new(PHYSICAL[0] as f32 * 0.75, PHYSICAL[1] as f32 * 0.6).unwrap();
            for _ in 0..32 {
                presentation
                    .build(
                        world.resource::<crate::player_runtime::PlayerRuntime>(),
                        &UiRuntime::new(1),
                        0,
                        PHYSICAL,
                        DpiScale::new(1.0).unwrap(),
                    )
                    .unwrap();
                assert_eq!(presentation.gui_scale_slider_track(), None);
                assert_eq!(
                    presentation.gui_scale_drag_action(UiPoint::new(0.0, 0.0).unwrap()),
                    None
                );
                if let Some(bounds) = presentation.menu_action_bounds(action) {
                    return bounds;
                }
                assert!(presentation.scroll_menu(pane, -20.0, false));
            }
            panic!("the native GUI-scale option enters the viewport after scrolling");
        })
}

fn pointer(app: &mut App, window: Entity, position: Vec2, event: Option<ButtonState>) {
    app.world_mut()
        .entity_mut(window)
        .get_mut::<Window>()
        .unwrap()
        .set_cursor_position(Some(position));
    if let Some(state) = event {
        app.world_mut().write_message(MouseButtonInput {
            button: MouseButton::Left,
            state,
            window,
        });
        let mut buttons = app.world_mut().resource_mut::<ButtonInput<MouseButton>>();
        match state {
            ButtonState::Pressed => buttons.press(MouseButton::Left),
            ButtonState::Released => buttons.release(MouseButton::Left),
        }
    }
    app.update();
}

#[test]
fn native_gui_scale_click_relayouts_and_held_pointer_never_drags_choices() {
    let Some(presentation) = engine_presentation() else {
        eprintln!(
            "skipping native_gui_scale_click_relayouts_and_held_pointer_never_drags_choices: missing installed local carriers (make assets)"
        );
        return;
    };
    let mut menu = MenuRuntime::new(true, 2, "Player".into());
    menu.set_gui_scale_preference(None);
    menu.activate(MenuAction::Navigate(MenuScreen::Settings));
    let mut app = App::new();
    app.add_plugins(MinimalPlugins)
        .add_message::<KeyboardInput>()
        .add_message::<MouseButtonInput>()
        .init_resource::<ButtonInput<KeyCode>>()
        .init_resource::<ButtonInput<MouseButton>>()
        .init_resource::<Touches>()
        .init_resource::<MenuClipboard>()
        .insert_resource(crate::player_runtime::PlayerRuntime::new(1))
        .insert_resource(menu)
        .insert_resource(presentation)
        .add_systems(Update, (drive_menu_input, apply_gui_scale_setting).chain());
    let window = app
        .world_mut()
        .spawn((
            Window {
                focused: true,
                resolution: WindowResolution::new(PHYSICAL[0], PHYSICAL[1]),
                ..Default::default()
            },
            CursorOptions::default(),
            PrimaryWindow,
        ))
        .id();
    app.update();
    let bounds = frame(&mut app, MenuAction::SettingsScale(-1));
    app.world_mut()
        .resource_mut::<MenuRuntime>()
        .refresh_settings_focus([MenuAction::SettingsScale(0)]);
    let centre = Vec2::new(
        (bounds.min().x() + bounds.max().x()) * 0.5,
        (bounds.min().y() + bounds.max().y()) * 0.5,
    );
    pointer(&mut app, window, centre, Some(ButtonState::Pressed));
    assert_eq!(app.world().resource::<MenuRuntime>().gui_scale_offset(), 0);
    pointer(&mut app, window, centre, Some(ButtonState::Released));
    assert_eq!(app.world().resource::<MenuRuntime>().gui_scale_offset(), -1);
    assert_eq!(
        app.world()
            .resource::<UiPresentationRuntime>()
            .gui_scale_preference(),
        Some(3)
    );
    let bounds = frame(&mut app, MenuAction::SettingsScale(-1));
    let centre = Vec2::new(
        (bounds.min().x() + bounds.max().x()) * 0.5,
        (bounds.min().y() + bounds.max().y()) * 0.5,
    );
    pointer(&mut app, window, centre, Some(ButtonState::Pressed));
    let inert = Vec2::new(PHYSICAL[0] as f32 * 0.75, 1.0);
    assert_eq!(
        app.world()
            .resource::<UiPresentationRuntime>()
            .hit_test_menu(UiPoint::new(inert.x, inert.y).unwrap()),
        None
    );
    pointer(&mut app, window, inert, None);
    assert_eq!(
        app.world().resource::<MenuRuntime>().gui_scale_offset(),
        -1,
        "holding the pointer across relayout does not capture option buttons"
    );
    assert_eq!(
        app.world().resource::<MenuRuntime>().view().focused_action,
        Some(MenuAction::SettingsScale(-1))
    );
    assert!(
        !app.world()
            .resource::<MenuRuntime>()
            .view()
            .navigation_focus_visible
    );
    pointer(&mut app, window, inert, Some(ButtonState::Released));
    let bounds = frame(&mut app, MenuAction::SettingsScale(0));
    let centre = Vec2::new(
        (bounds.min().x() + bounds.max().x()) * 0.5,
        (bounds.min().y() + bounds.max().y()) * 0.5,
    );
    pointer(&mut app, window, centre, None);
    assert_eq!(app.world().resource::<MenuRuntime>().gui_scale_offset(), -1);
    pointer(&mut app, window, centre, Some(ButtonState::Pressed));
    assert_eq!(app.world().resource::<MenuRuntime>().gui_scale_offset(), -1);
    pointer(&mut app, window, centre, Some(ButtonState::Released));
    assert_eq!(app.world().resource::<MenuRuntime>().gui_scale_offset(), 0);
    assert_eq!(
        app.world()
            .resource::<UiPresentationRuntime>()
            .gui_scale_preference(),
        Some(4)
    );
}
