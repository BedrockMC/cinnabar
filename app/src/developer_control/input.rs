//! Synthetic input: controls become Bevy keyboard and mouse messages, so the semantic router,
//! UI, menus and local mods see them exactly as real keys.

use std::collections::VecDeque;

use bevy::{
    ecs::message::MessageCursor,
    input::{
        ButtonState, InputSystems,
        keyboard::{Key, KeyboardInput, NativeKey},
        mouse::MouseButtonInput,
    },
    prelude::*,
    window::{CursorGrabMode, CursorOptions, PrimaryWindow, WindowFocused},
};
use developer_control::{
    input::{Control, InputPlan, MouseButton as ControlButton, yaw_difference},
    protocol::{InputCommand, Look},
};
use semantic_input::PhysicalControl;
use serde_json::{Value, json};

use crate::{
    camera::{DrivenInput, FlyCameraUpdateSet, PITCH_LIMIT},
    local_player::{LocalPlayerFrameSet, LocalViewPose},
    runtime::telemetry::bedrock_camera_rotation,
    semantic_controls::physical::{KEYBOARD_USAGES, mouse_button_code},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Physical {
    Key(KeyCode),
    Mouse(MouseButton),
}

#[derive(Debug, Clone, Copy)]
struct LookTween {
    from: [f32; 2],
    to: [f32; 2],
    frame: u32,
    frames: u32,
}

#[derive(Resource, Default)]
pub(super) struct Driver {
    /// Held controls in press order; a tap carries the frames it has left.
    held: Vec<(Physical, Option<u32>)>,
    events: VecDeque<(Physical, ButtonState)>,
    look: Option<Look>,
    tween: Option<LookTween>,
    /// The OS focus last reported, restored when control is handed back.
    real_focus: Option<bool>,
    focus_cursor: MessageCursor<WindowFocused>,
}

pub(super) fn configure(app: &mut App) {
    app.init_resource::<Driver>()
        .add_systems(PreUpdate, inject.before(InputSystems))
        .add_systems(
            Update,
            apply_look
                .after(FlyCameraUpdateSet)
                .before(LocalPlayerFrameSet::Physics),
        );
}

pub(super) fn apply(world: &mut World, command: &InputCommand) -> Result<Value, String> {
    let plan = InputPlan::from_command(command)?;
    let menu = world.get_resource::<crate::menu::MenuRuntime>();
    let resolve = |controls: &[Control]| -> Result<Vec<Physical>, String> {
        controls
            .iter()
            .map(|control| resolve(menu, control))
            .collect()
    };
    let (release, hold, tap) = (
        resolve(&plan.release)?,
        resolve(&plan.hold)?,
        resolve(&plan.tap)?,
    );
    let mut driver = world.resource_mut::<Driver>();
    if plan.release_all {
        let held: Vec<_> = driver.held.iter().map(|(physical, _)| *physical).collect();
        for physical in held {
            driver.release(physical);
        }
        driver.tween = None;
    }
    for physical in release {
        driver.release(physical);
    }
    for physical in hold {
        driver.press(physical, None);
    }
    for physical in tap {
        driver.press(physical, Some(plan.tap_frames));
    }
    if plan.look.is_some() {
        driver.look = plan.look;
    }
    let held = driver
        .held
        .iter()
        .map(|(physical, _)| format!("{physical:?}"))
        .collect::<Vec<_>>();
    if plan.release_control {
        world.remove_resource::<DrivenInput>();
        return Ok(json!({ "driven": false }));
    }
    world.init_resource::<DrivenInput>();
    Ok(json!({ "driven": true, "held": held }))
}

impl Driver {
    fn press(&mut self, physical: Physical, frames: Option<u32>) {
        match self.held.iter_mut().find(|(held, _)| *held == physical) {
            Some((_, remaining)) => *remaining = frames,
            None => {
                self.held.push((physical, frames));
                self.events.push_back((physical, ButtonState::Pressed));
            }
        }
    }

    fn release(&mut self, physical: Physical) {
        let before = self.held.len();
        self.held.retain(|(held, _)| *held != physical);
        if self.held.len() != before {
            self.events.push_back((physical, ButtonState::Released));
        }
    }
}

fn resolve(menu: Option<&crate::menu::MenuRuntime>, control: &Control) -> Result<Physical, String> {
    match control {
        Control::Key(name) => KEYBOARD_USAGES
            .iter()
            .find(|(key, _)| format!("{key:?}") == *name)
            .map(|(key, _)| Physical::Key(*key))
            .ok_or_else(|| format!("unknown key `{name}`")),
        Control::Mouse(button) => Ok(Physical::Mouse(match button {
            ControlButton::Left => MouseButton::Left,
            ControlButton::Right => MouseButton::Right,
            ControlButton::Middle => MouseButton::Middle,
            ControlButton::Back => MouseButton::Back,
            ControlButton::Forward => MouseButton::Forward,
        })),
        Control::Binding(name) => match crate::menu::settings_options::named_control(menu, name) {
            Some(PhysicalControl::KeyboardUsage(usage)) => KEYBOARD_USAGES
                .iter()
                .find(|(_, candidate)| *candidate == usage)
                .map(|(key, _)| Physical::Key(*key))
                .ok_or_else(|| format!("`{name}` is bound to an unmapped key")),
            Some(PhysicalControl::MouseButton(code)) => [
                MouseButton::Left,
                MouseButton::Right,
                MouseButton::Middle,
                MouseButton::Back,
                MouseButton::Forward,
                MouseButton::Other(u16::from(code.saturating_sub(1))),
            ]
            .into_iter()
            .find(|button| mouse_button_code(*button) == Some(code))
            .map(Physical::Mouse)
            .ok_or_else(|| format!("`{name}` is bound to an unknown mouse button")),
            Some(other) => Err(format!("`{name}` is bound to {other:?}, not a key")),
            None => Err(format!("unknown binding `{name}`")),
        },
    }
}

/// Emits queued edges and expires taps; while driven, the window counts as focused and
/// captured without the OS cursor ever being grabbed.
#[allow(clippy::too_many_arguments)]
fn inject(
    driven: Option<Res<DrivenInput>>,
    mut driver: ResMut<Driver>,
    focus: Res<Messages<WindowFocused>>,
    mut window: Query<(Entity, &mut Window, &mut CursorOptions), With<PrimaryWindow>>,
    mut keys: MessageWriter<KeyboardInput>,
    mut buttons: MessageWriter<MouseButtonInput>,
    mut was_driven: Local<bool>,
) {
    let Ok((entity, mut window, mut cursor)) = window.single_mut() else {
        return;
    };
    let mut focus_cursor = std::mem::take(&mut driver.focus_cursor);
    if let Some(event) = focus_cursor.read(&focus).last() {
        driver.real_focus = Some(event.focused);
    }
    driver.focus_cursor = focus_cursor;
    let driving = driven.is_some();
    if driving {
        let window = window.bypass_change_detection();
        driver.real_focus.get_or_insert(window.focused);
        window.focused = true;
        let cursor = cursor.bypass_change_detection();
        cursor.grab_mode = CursorGrabMode::Locked;
        cursor.visible = false;
    } else if std::mem::take(&mut *was_driven) {
        window.bypass_change_detection().focused = driver.real_focus.unwrap_or(false);
        let cursor = cursor.bypass_change_detection();
        cursor.grab_mode = CursorGrabMode::None;
        cursor.visible = true;
    }
    *was_driven = driving;
    let expired: Vec<_> = driver
        .held
        .iter()
        .filter(|(_, frames)| *frames == Some(0))
        .map(|(physical, _)| *physical)
        .collect();
    for physical in expired {
        driver.release(physical);
    }
    while let Some((physical, state)) = driver.events.pop_front() {
        match physical {
            Physical::Key(key_code) => {
                keys.write(KeyboardInput {
                    key_code,
                    logical_key: Key::Unidentified(NativeKey::Unidentified),
                    state,
                    text: None,
                    repeat: false,
                    window: entity,
                });
            }
            Physical::Mouse(button) => {
                buttons.write(MouseButtonInput {
                    button,
                    state,
                    window: entity,
                });
            }
        }
    }
    for (_, frames) in &mut driver.held {
        if let Some(frames) = frames {
            *frames = frames.saturating_sub(1);
        }
    }
}

/// Bedrock yaw and pitch of the player's view, in degrees.
pub(super) fn view_angles(view: &LocalViewPose) -> [f32; 2] {
    let (yaw, pitch, _) = view.rotation().to_euler(EulerRot::YXZ);
    [
        (180.0 - yaw.to_degrees()).rem_euclid(360.0),
        -pitch.to_degrees(),
    ]
}

/// Turns the player's own view, as mouse look would, so movement and the server follow it.
fn apply_look(mut driver: ResMut<Driver>, mut view: ResMut<LocalViewPose>) {
    if let Some(look) = driver.look.take() {
        let from = view_angles(&view);
        let to = if look.relative {
            [from[0] + look.yaw, from[1] + look.pitch]
        } else {
            [look.yaw, look.pitch]
        };
        let limit = PITCH_LIMIT.to_degrees();
        let to = [
            from[0] + yaw_difference(from[0], to[0]),
            to[1].clamp(-limit, limit),
        ];
        driver.tween = Some(LookTween {
            from,
            to,
            frame: 0,
            frames: look.frames,
        });
    }
    let Some(tween) = driver.tween.as_mut() else {
        return;
    };
    tween.frame = tween.frame.saturating_add(1);
    let t = if tween.frames == 0 {
        1.0
    } else {
        (tween.frame as f32 / tween.frames as f32).min(1.0)
    };
    let yaw = tween.from[0] + (tween.to[0] - tween.from[0]) * t;
    let pitch = tween.from[1] + (tween.to[1] - tween.from[1]) * t;
    view.set_rotation(bedrock_camera_rotation(yaw, pitch));
    if t >= 1.0 {
        driver.tween = None;
    }
}
