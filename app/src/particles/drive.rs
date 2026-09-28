use assets::NetworkIdMode;
use bevy::camera::Projection;
use bevy::prelude::{App, Local, Query, Res, ResMut, Resource, Time, Transform, Update, With};
use client_world::{CommittedParticleEvent, WorldStream};
use protocol::{ParticleEvent, SpawnParticleEffectEvent};
use render::{
    AtmosphereFrame, LevelParticle, ParticleGpuFrame, ParticleSystem, SpawnRequest,
    block_break_request, block_crack_request, classify_level_event, named_request,
    parse_molang_variables, particle_view, update_particle_frame,
};

use super::{tiles::block_tile, world_adapter::StreamParticleWorld};
use crate::{camera::FlyCamera, movement::PhysicsCollisionRegistries, runtime::world::ClientWorld};

/// Committed particle triggers waiting for the next frame's drive.
#[derive(Resource, Debug, Default)]
pub(crate) struct ParticleInbox {
    events: Vec<CommittedParticleEvent>,
}

/// Seconds between hit-particle bursts on a block being mined; needs independent measurement.
const CRACK_INTERVAL_SECONDS: f32 = 0.2;
const MAX_CRACKING_BLOCKS: usize = 8;

const WHITE: [f32; 4] = [1.0; 4];
const BLOCK_BREAK_EFFECT: &str = "minecraft:block_destruct";
/// Height fraction of an actor's box where critical-hit pieces originate.
const CRITICAL_HEIGHT_FRACTION: f32 = 0.9;

pub(crate) fn drain_committed_particles(stream: &mut WorldStream, inbox: &mut ParticleInbox) {
    inbox.events.extend(stream.take_committed_particles());
    if inbox.events.len() > 512 {
        let excess = inbox.events.len() - 512;
        inbox.events.drain(..excess);
    }
}

pub(crate) fn configure_particles(app: &mut App) {
    app.init_resource::<ParticleInbox>().add_systems(
        Update,
        drive_particles.after(crate::camera::FlyCameraUpdateSet),
    );
}

fn floor_cell(position: [f32; 3]) -> [i32; 3] {
    position.map(|c| c.floor() as i32)
}

fn spawn_spawn_packet(
    system: &mut ParticleSystem,
    stream: &WorldStream,
    event: &SpawnParticleEffectEvent,
) {
    let mut position = event.position;
    if let Some(unique_id) = event.actor_unique_id {
        // The position is relative to the attached actor.
        let Some(actor) = stream.actor_by_unique_id(unique_id) else {
            return;
        };
        for axis in 0..3 {
            position[axis] += actor.position[axis];
        }
    }
    let variables = event
        .molang_variables
        .as_deref()
        .map(parse_molang_variables)
        .unwrap_or_default();
    system.spawn(&SpawnRequest {
        effect: event.effect.to_string(),
        position,
        variables,
        ..SpawnRequest::default()
    });
}

fn route_level_event(
    system: &mut ParticleSystem,
    world: &StreamParticleWorld<'_>,
    stream: &WorldStream,
    mode: NetworkIdMode,
    event_id: i32,
    position: [f32; 3],
    data: i32,
) {
    let assets = stream.runtime_assets();
    match classify_level_event(event_id, data) {
        Some(LevelParticle::Named {
            effect,
            spell_color,
        }) => {
            system.spawn(&named_request(effect, position, spell_color));
        }
        Some(LevelParticle::BlockBreak { runtime_id })
        | Some(LevelParticle::Terrain { runtime_id }) => {
            if let Some(tile) = block_tile(assets, mode, runtime_id as u32) {
                system.spawn(&block_break_request(
                    BLOCK_BREAK_EFFECT,
                    floor_cell(position),
                    tile,
                    WHITE,
                ));
            }
        }
        Some(LevelParticle::BlockCrack { face, .. }) => {
            spawn_crack(system, world, assets, mode, floor_cell(position), face);
        }
        None => {}
    }
}

fn spawn_crack(
    system: &mut ParticleSystem,
    world: &StreamParticleWorld<'_>,
    assets: &assets::RuntimeAssets,
    mode: NetworkIdMode,
    block: [i32; 3],
    face: u8,
) {
    let Some(runtime_id) = world.block_runtime_id(block) else {
        return;
    };
    if let Some(tile) = block_tile(assets, mode, runtime_id) {
        system.spawn(&block_crack_request(
            BLOCK_BREAK_EFFECT,
            block,
            face,
            tile,
            WHITE,
        ));
    }
}

/// Face index (0 down, 1 up, 2 north, 3 south, 4 west, 5 east) of `block` nearest the camera.
fn face_toward(block: [i32; 3], camera: [f32; 3]) -> u8 {
    let delta: [f32; 3] = std::array::from_fn(|i| camera[i] - (block[i] as f32 + 0.5));
    let axis = (0..3)
        .max_by(|&a, &b| delta[a].abs().total_cmp(&delta[b].abs()))
        .unwrap_or(1);
    match (axis, delta[axis] >= 0.0) {
        (0, true) => 5,
        (0, false) => 4,
        (1, true) => 1,
        (1, false) => 0,
        (_, true) => 3,
        (_, false) => 2,
    }
}

/// Hit pieces on the nearest blocks the server reports as being cracked.
fn spawn_mining_cracks(
    system: &mut ParticleSystem,
    world: &StreamParticleWorld<'_>,
    stream: &WorldStream,
    mode: NetworkIdMode,
    camera: [f32; 3],
) {
    let snapshot = stream.block_crack_snapshot();
    let mut blocks: Vec<[i32; 3]> = snapshot
        .entries
        .iter()
        .map(|crack| crack.position)
        .collect();
    let distance = |block: &[i32; 3]| -> f32 {
        (0..3)
            .map(|i| (block[i] as f32 + 0.5 - camera[i]).powi(2))
            .sum()
    };
    blocks.sort_by(|a, b| distance(a).total_cmp(&distance(b)));
    for block in blocks.into_iter().take(MAX_CRACKING_BLOCKS) {
        spawn_crack(
            system,
            world,
            stream.runtime_assets(),
            mode,
            block,
            face_toward(block, camera),
        );
    }
}

fn route_critical(system: &mut ParticleSystem, stream: &WorldStream, runtime_id: u64, magic: bool) {
    let Some(actor) = stream.actor(runtime_id) else {
        return;
    };
    let height = actor
        .bounding_box()
        .map_or(1.8, |(min, max)| max[1] - min[1]);
    let mut position = actor.position;
    position[1] += height * CRITICAL_HEIGHT_FRACTION;
    let effect = if magic {
        "minecraft:magic_critical_hit_emitter"
    } else {
        "minecraft:critical_hit_emitter"
    };
    system.spawn(&named_request(effect, position, None));
}

#[allow(clippy::too_many_arguments)]
fn drive_particles(
    time: Res<Time>,
    mut inbox: ResMut<ParticleInbox>,
    mut system: ResMut<ParticleSystem>,
    mut frame: ResMut<ParticleGpuFrame>,
    client_world: Res<ClientWorld>,
    collisions: Res<PhysicsCollisionRegistries>,
    atmosphere: Res<AtmosphereFrame>,
    cameras: Query<(&Transform, &Projection), With<FlyCamera>>,
    mut session: Local<(u64, i32)>,
    mut crack_timer: Local<f32>,
) {
    let Some(stream) = client_world.stream.as_ref() else {
        if system.emitter_count() > 0 {
            system.clear();
        }
        inbox.events.clear();
        return;
    };
    let Ok((transform, projection)) = cameras.single() else {
        return;
    };
    let identity = (stream.actor_session_id(), stream.current_dimension());
    if *session != identity {
        *session = identity;
        system.clear();
        inbox.events.retain(|event| event.dimension == identity.1);
    }
    let mode = stream.network_id_mode();
    let world = StreamParticleWorld::new(stream, collisions.registry(mode));
    let view = particle_view(&(*transform).into(), projection);
    system.set_camera(view.position);
    system.daylight = (atmosphere.sun_direction()[1] * 0.8 + 0.2).clamp(0.0, 1.0);

    for committed in inbox.events.drain(..) {
        match &committed.event {
            ParticleEvent::Level(level) => route_level_event(
                &mut system,
                &world,
                stream,
                mode,
                level.event_id,
                level.position,
                level.data,
            ),
            ParticleEvent::Spawn(spawn) => spawn_spawn_packet(&mut system, stream, spawn),
            ParticleEvent::ActorCritical {
                actor_runtime_id,
                magic,
            } => route_critical(&mut system, stream, *actor_runtime_id, *magic),
        }
    }
    *crack_timer += time.delta_secs();
    if *crack_timer >= CRACK_INTERVAL_SECONDS {
        *crack_timer = 0.0;
        spawn_mining_cracks(&mut system, &world, stream, mode, view.position);
    }
    update_particle_frame(&mut system, &mut frame, time.delta_secs(), &view, &world);
}
