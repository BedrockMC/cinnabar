use super::*;
use crate::named_audio::tests::sample;

#[test]
fn actual_mixer_stall_keeps_all_cancelled_permits_until_node_drop() {
    let sample = sample();
    let pool = Arc::new(PermitPool::default());
    let (controller, mut mixer) = rodio::dynamic_mixer::mixer::<f32>(2, 44100);
    let mut controls = Vec::new();
    for _ in 0..VOICE_LIMIT {
        let (source, control) = CancelablePcm::prepare(&sample, &pool).unwrap();
        controller.add(source.convert_samples::<f32>());
        controls.push(control);
    }
    for control in &controls {
        control.cancel();
    }
    assert_eq!(pool.occupied(), 16);
    assert!(CancelablePcm::prepare(&sample, &pool).is_none());
    // Uniform/resampling may retain a bounded buffered tail; don't assert zero latency.
    for _ in 0..1024 {
        mixer.next();
        if pool.occupied() == 0 {
            break;
        }
    }
    assert_eq!(pool.occupied(), 0);
    for control in controls {
        assert!(pool.retired(control.slot, control.token));
    }
}
#[test]
fn actual_uniform_retains_naturally_exhausted_source_until_outer_drop() {
    let sample = sample();
    let pool = Arc::new(PermitPool::default());
    let (source, _) = CancelablePcm::prepare(&sample, &pool).unwrap();
    let mut outer = rodio::source::UniformSourceIterator::<_, f32>::new(source, 1, 32000);
    let mut exhausted = false;
    for _ in 0..1024 {
        if outer.next().is_none() {
            exhausted = true;
            break;
        }
    }
    assert!(
        exhausted,
        "qualified finite sample must exhaust within bound"
    );
    assert_eq!(pool.occupied(), 1, "None is not backend Drop");
    drop(outer);
    assert_eq!(pool.occupied(), 0);
}
#[test]
fn mixer_and_controller_drop_release_pending_and_active_nodes() {
    let sample = sample();
    let pool = Arc::new(PermitPool::default());
    let (controller, mut mixer) = rodio::dynamic_mixer::mixer::<f32>(2, 48000);
    for _ in 0..2 {
        let (source, _) = CancelablePcm::prepare(&sample, &pool).unwrap();
        controller.add(source.convert_samples::<f32>());
    }
    mixer.next();
    let (source, _) = CancelablePcm::prepare(&sample, &pool).unwrap();
    controller.add(source.convert_samples::<f32>());
    assert!(pool.occupied() > 0);
    drop(mixer);
    drop(controller);
    assert_eq!(pool.occupied(), 0);
}
#[test]
fn repeated_cancellation_cycles_and_no_device_cannot_remint_or_leak() {
    let sample = sample();
    let pool = Arc::new(PermitPool::default());
    let mut device = AudioDevice::disabled();
    for _ in 0..128 {
        let (source, control) = CancelablePcm::prepare(&sample, &pool).unwrap();
        assert!(!device.submit(source));
        assert!(pool.retired(control.slot, control.token));
        assert_eq!(pool.occupied(), 0);
    }
    pool.slots[0].store(u64::MAX - 1, Ordering::Release);
    let mut permits = Vec::new();
    for _ in 0..15 {
        permits.push(pool.acquire().unwrap());
    }
    assert!(pool.acquire().is_none());
    drop(permits);
    assert_eq!(pool.occupied(), 0);
}

#[test]
fn actual_mixer_repeated_retirement_reuses_capacity_without_old_control_cancelling_new_source() {
    let sample = sample();
    let pool = Arc::new(PermitPool::default());
    let (controller, mut mixer) = rodio::dynamic_mixer::mixer::<f32>(2, 44100);
    let mut previous: Option<VoiceControl> = None;
    for _ in 0..64 {
        let (source, control) = CancelablePcm::prepare(&sample, &pool).unwrap();
        if let Some(old) = previous.take() {
            assert!(pool.retired(old.slot, old.token));
            old.cancel();
            assert!(!control.cancel.load(Ordering::Acquire));
        }
        controller.add(source.convert_samples::<f32>());
        assert_eq!(pool.occupied(), 1);
        control.cancel();
        for _ in 0..1024 {
            mixer.next();
            if pool.occupied() == 0 {
                break;
            }
        }
        assert_eq!(pool.occupied(), 0);
        previous = Some(control);
    }
}

use crate::named_audio::{NamedAudio, SequencedAudioEvent, drain_live_named_audio};

fn play(sequence: u64) -> SequencedAudioEvent {
    SequencedAudioEvent {
        origin_stream_session_id: 1,
        sequence,
        dimension: 0,
        dimension_epoch: 0,
        event: protocol::AudioEvent::Play(protocol::PlayAudioEvent {
            name: Arc::from("ambient.underwater.loop"),
            position: [0; 3],
            volume: 1.0,
            pitch: 1.0,
            loop_count: -1,
            server_sound_handle: None,
        }),
    }
}
fn stop(sequence: u64, all: bool) -> SequencedAudioEvent {
    SequencedAudioEvent {
        origin_stream_session_id: 1,
        sequence,
        dimension: 0,
        dimension_epoch: 0,
        event: protocol::AudioEvent::Stop(protocol::StopAudioEvent {
            name: Arc::from("ambient.underwater.loop"),
            stop_all_sounds: all,
            stop_music_legacy: false,
        }),
    }
}

// Independently generated PCM is decoded through the real validated carrier.
// These composed tests prove routing/ownership, not authentic audible content.
struct EmptyAudioWorld;
impl sim::CollisionWorld for EmptyAudioWorld {
    fn collision_boxes(
        &self,
        _: sim::Aabb,
    ) -> Result<sim::CollisionQuery<Vec<sim::Aabb>>, sim::WorldQueryError> {
        Ok(sim::CollisionQuery::synthetic(Vec::new()))
    }
}
fn forward_live_audio(
    mut world: bevy::prelude::ResMut<crate::runtime::world::ClientWorld>,
    mut messages: bevy::prelude::MessageWriter<SequencedAudioEvent>,
) {
    if let Some(stream) = world.stream.as_mut() {
        crate::runtime::audio::drain_committed_audio(stream, |event| {
            messages.write(event);
        });
    }
}
fn composed_app() -> (bevy::prelude::App, rodio::dynamic_mixer::DynamicMixer<f32>) {
    use crate::{
        camera::{CameraSettingsAuthority, FlyCamera},
        environment::WorldClock,
        local_player::{
            CameraPose, LocalPlayerFrameCarrier, LocalViewPose, publish_local_player_frame,
            resolve_camera_pose,
        },
        local_player_camera_receipt::{CameraPublicationAttempt, begin_camera_publication_attempt},
        movement::{LocalPhysicsController, PhysicsCollisionRegistries},
        runtime::world::ClientWorld,
    };
    use bevy::prelude::*;
    let mut world = ClientWorld::new(Arc::new(assets::RuntimeAssets::diagnostic()));
    world.stream = Some(client_world::WorldStream::new(protocol::WorldBootstrap {
        dimension: 0,
        local_player_runtime_id: 1,
        local_player_unique_id: 1,
        player_position: [0.0, 64.0, 0.0],
        world_spawn_position: [0; 3],
        air_network_id: protocol::SEQUENTIAL_AIR_NETWORK_ID,
        block_network_ids_are_hashes: false,
    }));
    let collisions = PhysicsCollisionRegistries::bind_coherent_assets(
        crate::asset_startup::pinned_block_registry_bytes(),
        include_bytes!("../../../../crates/assets/data/block-physics-v2168.bin"),
        std::path::Path::new("fixture.preg"),
        std::path::Path::new("fixture.mcbea"),
        crate::asset_startup::active_content_registry_protocol(),
    )
    .unwrap();
    let mut physics = LocalPhysicsController::default();
    physics.reanchor_network_position([0.0, 64.0, 0.0], 0, false);
    physics.advance(
        std::time::Duration::from_millis(50),
        sim::MovementInput::default(),
        &EmptyAudioWorld,
    );
    assert!(physics.last_world_identity().is_some());
    let (device, mixer) = AudioDevice::test_mixer();
    let mut app = App::new();
    app.add_message::<SequencedAudioEvent>()
        .insert_resource(world)
        .insert_resource(collisions)
        .insert_resource(physics)
        .init_resource::<WorldClock>()
        .init_resource::<CameraPose>()
        .insert_resource(LocalViewPose::new(
            Vec3::new(0.0, 65.62, 0.0),
            Quat::IDENTITY,
        ))
        .init_resource::<CameraSettingsAuthority>()
        .init_resource::<LocalPlayerFrameCarrier>()
        .init_resource::<CameraPublicationAttempt>()
        .insert_resource(NamedAudio::new(Some(Arc::new(sample()))))
        .insert_non_send_resource(device)
        .add_systems(
            Update,
            (
                forward_live_audio,
                begin_camera_publication_attempt,
                resolve_camera_pose,
                publish_local_player_frame,
                drain_live_named_audio,
            )
                .chain(),
        );
    app.world_mut()
        .spawn((FlyCamera::default(), Transform::default()));
    (app, mixer)
}
fn commit(app: &mut bevy::prelude::App, sequence: u64, event: protocol::WorldEvent) {
    app.world_mut()
        .resource_mut::<crate::runtime::world::ClientWorld>()
        .stream
        .as_mut()
        .unwrap()
        .submit(sequence, event)
        .unwrap();
}
fn live_play() -> protocol::WorldEvent {
    let protocol::AudioEvent::Play(mut event) = play(1).event else {
        unreachable!()
    };
    event.position = [0, 512, 0];
    protocol::WorldEvent::Audio(protocol::AudioEvent::Play(event))
}
fn live_dimension(dimension: i32) -> protocol::WorldEvent {
    protocol::WorldEvent::ChangeDimension(protocol::ChangeDimensionEvent {
        dimension,
        position: [0.0, 64.0, 0.0],
    })
}

#[test]
fn actual_committed_ingress_camera_writer_publication_and_mixer_submit_admit_once() {
    let (mut app, mut mixer) = composed_app();
    commit(&mut app, 1, live_play());
    app.update();
    let proof = app
        .world()
        .resource::<crate::local_player_camera_receipt::CameraPublicationAttempt>()
        .published()
        .unwrap();
    assert_eq!(proof.owner.sequence, 1);
    let audio = app.world().resource::<NamedAudio>();
    assert_eq!(
        (
            audio.stats.accepted,
            audio.stats.submitted,
            audio.stats.missing_camera
        ),
        (1, 1, 0)
    );
    assert_eq!(audio.pool.occupied(), 1);
    assert!(mixer.by_ref().take(8).any(|value| value > 0.0));
    app.update();
    assert_eq!(
        app.world().resource::<NamedAudio>().stats.submitted,
        1,
        "live reader does not replay diagnostics"
    );
}

#[test]
fn delayed_actual_old_epoch_stop_cannot_cancel_current_epoch_submitted_voice() {
    let (mut app, mut mixer) = composed_app();
    commit(
        &mut app,
        1,
        protocol::WorldEvent::Audio(stop(1, true).event),
    );
    let mut held = Vec::new();
    {
        let mut world = app
            .world_mut()
            .resource_mut::<crate::runtime::world::ClientWorld>();
        crate::runtime::audio::drain_committed_audio(world.stream.as_mut().unwrap(), |event| {
            held.push(event)
        });
    }
    assert_eq!((held[0].dimension, held[0].dimension_epoch), (0, 0));
    commit(&mut app, 2, live_dimension(1));
    commit(&mut app, 3, live_dimension(0));
    commit(&mut app, 4, live_play());
    app.update();
    let proof = app
        .world()
        .resource::<crate::local_player_camera_receipt::CameraPublicationAttempt>()
        .published()
        .unwrap();
    assert_eq!((proof.owner.dimension, proof.owner.epoch), (0, 3));
    assert_eq!(app.world().resource::<NamedAudio>().stats.submitted, 1);
    assert!(mixer.by_ref().take(8).any(|value| value > 0.0));
    // Delayed delivery uses the actual producer envelope, not a forged epoch.
    app.world_mut().write_message(held.pop().unwrap());
    app.update();
    let audio = app.world().resource::<NamedAudio>();
    assert_eq!(
        (
            audio.stats.stale,
            audio.stats.stopped,
            audio.pool.occupied()
        ),
        (1, 0, 1)
    );
    assert!(mixer.by_ref().take(8).any(|value| value > 0.0));
    commit(
        &mut app,
        5,
        protocol::WorldEvent::Audio(stop(5, false).event),
    );
    app.update();
    assert_eq!(app.world().resource::<NamedAudio>().stats.stopped, 1);
    assert_eq!(app.world().resource::<NamedAudio>().pool.occupied(), 1);
    for _ in 0..1024 {
        mixer.next();
        if app.world().resource::<NamedAudio>().pool.occupied() == 0 {
            break;
        }
    }
    assert_eq!(app.world().resource::<NamedAudio>().pool.occupied(), 0);
}

#[test]
fn every_committed_camera_family_cancels_live_voice_without_reminting_or_regrant() {
    for event in [
        protocol::CameraEvent::Switch(protocol::CameraSwitchEvent {
            camera_unique_id: 1,
            target_player_unique_id: 1,
        }),
        protocol::CameraEvent::Instruction(protocol::CameraInstructionEvent::default()),
        protocol::CameraEvent::Instruction(protocol::CameraInstructionEvent {
            clear: Some(true),
            ..Default::default()
        }),
        protocol::CameraEvent::Instruction(protocol::CameraInstructionEvent {
            clear: Some(false),
            ..Default::default()
        }),
        protocol::CameraEvent::Shake(protocol::CameraShakeEvent {
            intensity: 0.1,
            duration_seconds: 0.1,
            shake_type: protocol::CameraShakeType::Positional,
            action: protocol::CameraShakeAction::Add,
        }),
        protocol::CameraEvent::Shake(protocol::CameraShakeEvent {
            intensity: 0.0,
            duration_seconds: 0.0,
            shake_type: protocol::CameraShakeType::Rotational,
            action: protocol::CameraShakeAction::Stop,
        }),
    ] {
        let (mut app, mut mixer) = composed_app();
        commit(&mut app, 1, live_play());
        app.update();
        assert_eq!(app.world().resource::<NamedAudio>().stats.submitted, 1);
        assert!(mixer.by_ref().take(8).any(|value| value > 0.0));
        commit(&mut app, 2, protocol::WorldEvent::Camera(event));
        app.update();
        let audio = app.world().resource::<NamedAudio>();
        assert_eq!(
            audio.pool.occupied(),
            1,
            "cancellation is not backend retirement"
        );
        assert_eq!(audio.controls.iter().flatten().count(), 1);
        assert!(
            audio
                .controls
                .iter()
                .flatten()
                .all(|control| control.cancel.load(std::sync::atomic::Ordering::Acquire))
        );
        for _ in 0..1024 {
            mixer.next();
            if app.world().resource::<NamedAudio>().pool.occupied() == 0 {
                break;
            }
        }
        assert_eq!(app.world().resource::<NamedAudio>().pool.occupied(), 0);
        app.world_mut()
            .resource_mut::<crate::runtime::world::ClientWorld>()
            .stream
            .as_mut()
            .unwrap()
            .begin_timed_session();
        commit(&mut app, 3, live_play());
        app.update();
        let audio = app.world().resource::<NamedAudio>();
        assert_eq!((audio.stats.submitted, audio.stats.missing_camera), (1, 1));
        assert_eq!(audio.pool.occupied(), 0);
    }
}
