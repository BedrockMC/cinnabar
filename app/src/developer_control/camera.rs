//! The scripted cinematic camera: overrides the rendered camera after every player and
//! server camera writer, leaving the player's own view and physics untouched.

use bevy::prelude::*;
use developer_control::camera::CameraPath;
use serde_json::{Value, json};

use crate::{
    app::ClientFrameSet,
    camera::{FlyCamera, projection_fov_radians},
    runtime::telemetry::bedrock_camera_rotation,
};

#[derive(Resource)]
pub(super) struct ScriptedCamera {
    path: CameraPath,
    /// Game time at the first sampled frame.
    started: Option<f32>,
    elapsed: f32,
    /// The player's own `hide_hand` setting, restored on release.
    restore_hide_hand: Option<i32>,
}

/// The vanilla video option the cinematic camera borrows to hide the hand.
const HIDE_HAND: &str = "hide_hand";

impl ScriptedCamera {
    pub(super) fn finished(&self) -> bool {
        self.path.finished(self.elapsed)
    }

    pub(super) fn summary(&self) -> Value {
        json!({
            "elapsed": self.elapsed,
            "duration": self.path.duration(),
            "looping": self.path.looping,
            "finished": self.finished(),
        })
    }
}

pub(super) fn configure(app: &mut App) {
    app.add_systems(
        Update,
        drive_camera
            .after(ClientFrameSet::Camera)
            .before(ClientFrameSet::Interaction),
    );
}

pub(super) fn start(world: &mut World, path: CameraPath) -> Result<Value, String> {
    path.validate()?;
    let duration = path.duration();
    let mut restore_hide_hand = world
        .remove_resource::<ScriptedCamera>()
        .and_then(|previous| previous.restore_hide_hand);
    if let Some(mut menu) = world.get_resource_mut::<crate::menu::MenuRuntime>() {
        if let Some(value) = restore_hide_hand.take() {
            menu.set_named_option(HIDE_HAND, value);
        }
        if path.hide_hand {
            restore_hide_hand = Some(menu.settings_snapshot().0.value(HIDE_HAND));
            menu.set_named_option(HIDE_HAND, 1);
        }
    }
    world.insert_resource(ScriptedCamera {
        path,
        started: None,
        elapsed: 0.0,
        restore_hide_hand,
    });
    Ok(json!({ "duration": duration }))
}

pub(super) fn release(world: &mut World) -> Result<Value, String> {
    let Some(scripted) = world.remove_resource::<ScriptedCamera>() else {
        return Ok(json!({ "released": false }));
    };
    if let (Some(value), Some(mut menu)) = (
        scripted.restore_hide_hand,
        world.get_resource_mut::<crate::menu::MenuRuntime>(),
    ) {
        menu.set_named_option(HIDE_HAND, value);
    }
    Ok(json!({ "released": true }))
}

/// Game time drives sampling, so a fixed-clock recording plays the path frame-exactly.
fn drive_camera(
    time: Res<Time>,
    scripted: Option<ResMut<ScriptedCamera>>,
    mut cameras: Query<(&mut Transform, &mut Projection), With<FlyCamera>>,
) {
    let Some(mut scripted) = scripted else {
        return;
    };
    let now = time.elapsed_secs();
    let started = *scripted.started.get_or_insert(now);
    scripted.elapsed = now - started;
    let Some(sample) = scripted.path.sample(scripted.elapsed) else {
        return;
    };
    for (mut transform, mut projection) in &mut cameras {
        *transform = Transform::from_translation(Vec3::from_array(sample.position))
            .with_rotation(bedrock_camera_rotation(sample.yaw, sample.pitch));
        if let (Some(fov), Projection::Perspective(perspective)) = (sample.fov, &mut *projection) {
            perspective.fov = projection_fov_radians(fov);
        }
    }
}
