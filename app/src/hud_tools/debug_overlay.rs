//! F3 debug overlay data: gathers client state into the two text columns.

use bevy::{prelude::*, time::Real};

use crate::{
    local_player::LocalPlayerFrameCarrier,
    movement::PhysicsCollisionRegistries,
    runtime::world::ClientWorld,
    ui_runtime::presentation::{DebugLines, UiPresentationRuntime},
};

/// How far the targeted-block ray reaches.
const TARGET_RANGE_BLOCKS: f64 = 20.0;
const FPS_SMOOTHING: f32 = 0.1;

#[derive(Resource, Default)]
pub(super) struct DebugOverlayState {
    visible: bool,
    fps: f32,
}

pub(super) fn configure(app: &mut App) {
    app.init_resource::<DebugOverlayState>()
        .add_systems(Update, publish_debug_overlay);
}

struct Target {
    block: [i32; 3],
    face: u8,
    runtime_id: u32,
}

struct Snapshot {
    fps: f32,
    dimension: i32,
    position: [f32; 3],
    direction: [f32; 3],
    /// `(block, sky)` light at the eye.
    light: (u8, u8),
    biome: Option<String>,
    target: Option<Target>,
}

fn publish_debug_overlay(
    keys: Res<ButtonInput<KeyCode>>,
    time: Res<Time<Real>>,
    mut state: ResMut<DebugOverlayState>,
    mut presentation: ResMut<UiPresentationRuntime>,
    client_world: Res<ClientWorld>,
    frame: Res<LocalPlayerFrameCarrier>,
    collisions: Option<Res<PhysicsCollisionRegistries>>,
) {
    if keys.just_pressed(KeyCode::F3) {
        state.visible = !state.visible;
    }
    let delta = time.delta_secs();
    if delta > 0.0 {
        let instant = 1.0 / delta;
        state.fps = if state.fps > 0.0 {
            state.fps + (instant - state.fps) * FPS_SMOOTHING
        } else {
            instant
        };
    }
    if !state.visible {
        presentation.set_debug_lines(None);
        return;
    }
    let lines = match (client_world.stream.as_ref(), frame.snapshot()) {
        (Some(stream), Some(frame)) => {
            let eye = frame.eye();
            let direction = frame.direction();
            let target = collisions.as_deref().and_then(|collisions| {
                let world = sim::PaletteWorld::new(
                    stream.collision_store(),
                    collisions.registry(stream.network_id_mode()),
                    stream.current_dimension(),
                );
                let vector = |value: Vec3| {
                    sim::Vec3::new(f64::from(value.x), f64::from(value.y), f64::from(value.z))
                };
                let hit = world
                    .block_interaction_ray_current(
                        vector(eye),
                        vector(direction),
                        TARGET_RANGE_BLOCKS,
                    )
                    .ok()??;
                Some(Target {
                    block: hit.block_pos,
                    face: hit.face,
                    runtime_id: hit.runtime_id,
                })
            });
            let biome = stream.camera_biome_id(eye.to_array()).map(|id| {
                stream
                    .biome_definitions_snapshot()
                    .iter()
                    .find(|definition| u32::from(definition.biome_id.unwrap_or(u16::MAX)) == id)
                    .map_or_else(
                        || format!("id {id}"),
                        |definition| definition.name.to_string(),
                    )
            });
            format_lines(&Snapshot {
                fps: state.fps,
                dimension: stream.current_dimension(),
                position: frame.pose().translation.to_array(),
                direction: direction.to_array(),
                light: stream.light_level_at(eye.to_array()),
                biome,
                target,
            })
        }
        _ => DebugLines {
            left: vec![fps_line(state.fps)],
            right: Vec::new(),
        },
    };
    presentation.set_debug_lines(Some(lines));
}

fn fps_line(fps: f32) -> String {
    format!("{} fps", fps.round() as u32)
}

fn dimension_name(dimension: i32) -> &'static str {
    match dimension {
        0 => "Overworld",
        1 => "Nether",
        2 => "The End",
        _ => "Unknown",
    }
}

fn face_name(face: u8) -> &'static str {
    ["down", "up", "north", "south", "west", "east"]
        .get(usize::from(face))
        .copied()
        .unwrap_or("unknown")
}

/// Cardinal heading of a view direction plus its axis hint.
fn facing(direction: [f32; 3]) -> (&'static str, &'static str) {
    let [x, _, z] = direction;
    if x.abs() > z.abs() {
        if x > 0.0 {
            ("east", "Towards positive X")
        } else {
            ("west", "Towards negative X")
        }
    } else if z > 0.0 {
        ("south", "Towards positive Z")
    } else {
        ("north", "Towards negative Z")
    }
}

fn format_lines(snapshot: &Snapshot) -> DebugLines {
    let [x, y, z] = snapshot.position;
    let block = [x, y, z].map(|value| value.floor() as i32);
    let chunk = block.map(|value| value.div_euclid(16));
    let within = block.map(|value| value.rem_euclid(16));
    let [dx, dy, dz] = snapshot.direction;
    let yaw = (-dx).atan2(dz).to_degrees();
    // Adding zero turns a negative zero into a printable 0.0.
    let pitch = -dy.clamp(-1.0, 1.0).asin().to_degrees() + 0.0;
    let (heading, axis) = facing(snapshot.direction);
    let (block_light, sky_light) = snapshot.light;
    let mut left = vec![
        "Cinnabar".to_owned(),
        fps_line(snapshot.fps),
        format!("Dimension: {}", dimension_name(snapshot.dimension)),
        format!("XYZ: {x:.3} / {y:.5} / {z:.3}"),
        format!("Block: {} {} {}", block[0], block[1], block[2]),
        format!(
            "Chunk: {} {} {} in {} {} {}",
            within[0], within[1], within[2], chunk[0], chunk[1], chunk[2]
        ),
        format!("Facing: {heading} ({axis}) ({yaw:.1} / {pitch:.1})"),
        format!(
            "Client Light: {} ({sky_light} sky, {block_light} block)",
            block_light.max(sky_light)
        ),
    ];
    if let Some(biome) = &snapshot.biome {
        left.push(format!("Biome: {biome}"));
    }
    let right = snapshot.target.iter().flat_map(|target| {
        [
            format!(
                "Targeted Block: {}, {}, {}",
                target.block[0], target.block[1], target.block[2]
            ),
            format!("Block runtime id: {}", target.runtime_id),
            format!("Face: {}", face_name(target.face)),
        ]
    });
    DebugLines {
        left,
        right: right.collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snapshot() -> Snapshot {
        Snapshot {
            fps: 59.6,
            dimension: 0,
            position: [-1.5, 64.0, 17.25],
            direction: [0.0, 0.0, -1.0],
            light: (3, 15),
            biome: Some("plains".to_owned()),
            target: Some(Target {
                block: [-2, 63, 17],
                face: 1,
                runtime_id: 42,
            }),
        }
    }

    #[test]
    fn lines_report_position_chunk_facing_and_light() {
        let lines = format_lines(&snapshot());
        assert_eq!(lines.left[1], "60 fps");
        assert_eq!(lines.left[3], "XYZ: -1.500 / 64.00000 / 17.250");
        assert_eq!(lines.left[4], "Block: -2 64 17");
        assert_eq!(lines.left[5], "Chunk: 14 0 1 in -1 4 1");
        assert!(
            lines.left[6].starts_with("Facing: north (Towards negative Z) (-180.0 / 0.0)")
                || lines.left[6].starts_with("Facing: north (Towards negative Z) (180.0 / 0.0)")
        );
        assert_eq!(lines.left[7], "Client Light: 15 (15 sky, 3 block)");
        assert_eq!(lines.left[8], "Biome: plains");
    }

    #[test]
    fn targeted_block_fills_the_right_column() {
        let lines = format_lines(&snapshot());
        assert_eq!(lines.right[0], "Targeted Block: -2, 63, 17");
        assert_eq!(lines.right[2], "Face: up");
        let mut none = snapshot();
        none.target = None;
        assert!(format_lines(&none).right.is_empty());
    }

    #[test]
    fn facing_picks_the_dominant_horizontal_axis() {
        assert_eq!(facing([1.0, 0.0, 0.2]).0, "east");
        assert_eq!(facing([-1.0, 0.0, 0.2]).0, "west");
        assert_eq!(facing([0.1, 0.0, 1.0]).0, "south");
    }
}
