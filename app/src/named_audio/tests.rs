use super::*;
use sha2::{Digest, Sha256};
pub(super) fn sample() -> RuntimeAudioPcm {
    let samples: Vec<i16> = (0..64).map(|value| value * 100).collect();
    let pcm: Vec<_> = samples
        .iter()
        .flat_map(|value| value.to_le_bytes())
        .collect();
    let definition = assets::AudioDefinition {
        identifier: "ambient.underwater.loop".into(),
        category: None,
        subtitle: None,
        min_distance: None,
        max_distance: None,
        volume: None,
        pitch: None,
        use_legacy_max_distance: None,
        alternatives: vec![assets::AudioAlternative {
            object_form: true,
            name: "sounds/test/finite".into(),
            weight: 1,
            volume: None,
            pitch: None,
            is_3d: Some(false),
            stream: Some(true),
            load_on_low_memory: None,
        }]
        .into_boxed_slice(),
    };
    let catalog_bytes = assets::encode_audio_catalog([1; 32], [2; 32], &[definition]).unwrap();
    let expected = assets::AudioPcmExpectedIdentity::new(
        "ambient.underwater.loop",
        "sounds/test/finite.fsb",
        Sha256::digest(&catalog_bytes).into(),
        [1; 32],
        [2; 32],
        [3; 32],
        Sha256::digest(pcm).into(),
        208,
        2,
        48000,
        32,
    )
    .unwrap();
    let bytes = assets::encode_audio_pcm(&expected, &samples).unwrap();
    RuntimeAudioPcm::decode(
        &bytes,
        &assets::RuntimeAudioCatalog::decode(&catalog_bytes).unwrap(),
        expected.catalog_sha256(),
        &expected,
    )
    .unwrap()
}
fn state() -> NamedAudio {
    let mut value = NamedAudio::new(Some(Arc::new(sample())));
    value.bind(Some(AudioOwner {
        session: 0,
        stream: 1,
        dimension: 0,
        epoch: 0,
    }));
    value
}
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
#[test]
fn same_batch_stop_dominates_pending_and_stop_then_play_is_fresh() {
    let mut value = state();
    value.event(&play(1), Some([0.0; 3]), true);
    value.event(&stop(2, false), None, true);
    let mut submitted = 0;
    value.flush(|_| {
        submitted += 1;
        true
    });
    assert_eq!(submitted, 0);
    assert_eq!(value.pool.occupied(), 0);
    value.event(&stop(3, true), None, true);
    value.event(&play(4), Some([0.0; 3]), true);
    value.flush(|_| {
        submitted += 1;
        true
    });
    assert_eq!(submitted, 1);
}
#[test]
fn cancelled_submitted_sources_survive_session_reset_and_exhaust_capacity() {
    let mut value = state();
    let mut stalled = Vec::new();
    for sequence in 1..=16 {
        value.event(&play(sequence), Some([0.0; 3]), true);
    }
    value.flush(|source| {
        stalled.push(source);
        true
    });
    value.event(&stop(17, true), None, true);
    assert_eq!(value.pool.occupied(), 16);
    value.bind(None);
    value.bind(Some(AudioOwner {
        session: 1,
        stream: 1,
        dimension: 0,
        epoch: 0,
    }));
    value.event(&play(18), Some([0.0; 3]), true);
    assert_eq!(value.stats.capacity, 1);
    for source in &mut stalled {
        assert!(source.next().is_none());
    }
    assert_eq!(value.pool.occupied(), 16);
    drop(stalled);
    value.collect_retired();
    assert_eq!(value.pool.occupied(), 0);
}
#[test]
fn wrong_origin_epoch_duplicate_dynamics_camera_and_device_fail_closed() {
    let mut value = state();
    let mut event = play(1);
    event.dimension_epoch = 2;
    value.event(&event, Some([0.0; 3]), true);
    event = play(2);
    event.origin_stream_session_id = 2;
    value.event(&event, Some([0.0; 3]), true);
    event = play(3);
    if let protocol::AudioEvent::Play(play) = &mut event.event {
        play.loop_count = 0;
    }
    value.event(&event, Some([0.0; 3]), true);
    value.event(&play(4), None, true);
    value.event(&play(5), Some([0.0; 3]), false);
    value.event(&play(5), Some([0.0; 3]), true);
    assert_eq!(
        (
            value.stats.stale,
            value.stats.unsupported,
            value.stats.missing_camera,
            value.stats.unavailable
        ),
        (3, 1, 1, 1)
    );
    assert_eq!(value.pool.occupied(), 0);
    value.event(&play(6), Some([0.0; 3]), true);
    value.flush(|_| false);
    assert_eq!(value.stats.backend_failed, 1);
    assert_eq!(value.pool.occupied(), 0);
}
#[test]
fn cutoff_uses_three_axes_eighth_coordinates_and_strict_boundary() {
    for axis in 0..3 {
        let mut raw = [0; 3];
        raw[axis] = 128;
        assert!(!inside_radius(raw, [0.0; 3]));
        raw[axis] = 127;
        assert!(inside_radius(raw, [0.0; 3]));
        raw[axis] = -128;
        assert!(!inside_radius(raw, [0.0; 3]));
    }
    assert!(!inside_radius([i32::MAX; 3], [0.0; 3]));
    assert!(!inside_radius([0; 3], [f32::NAN; 3]));
    assert!(inside_radius([8, -16, 24], [1.0, -2.0, 3.0]));
}

#[test]
fn only_unit_finite_unhandled_no_loop_dynamics_are_admitted_and_stop_survives_capacity() {
    let mut value = state();
    for (index, (volume, pitch, loops, handle)) in [
        (0.2, 1.0, -1, None),
        (f32::NAN, 1.0, -1, None),
        (1.0, 0.5, -1, None),
        (1.0, f32::INFINITY, -1, None),
        (1.0, 1.0, -2, None),
        (1.0, 1.0, 1, None),
        (1.0, 1.0, -1, Some(1)),
    ]
    .into_iter()
    .enumerate()
    {
        let mut event = play(index as u64 + 1);
        if let protocol::AudioEvent::Play(play) = &mut event.event {
            play.volume = volume;
            play.pitch = pitch;
            play.loop_count = loops;
            play.server_sound_handle = handle;
        }
        value.event(&event, Some([0.0; 3]), true);
    }
    assert_eq!(value.stats.unsupported, 7);
    assert_eq!(value.pool.occupied(), 0);
    for sequence in 8..24 {
        value.event(&play(sequence), Some([0.0; 3]), true);
    }
    value.event(&play(24), Some([0.0; 3]), true);
    assert_eq!(value.stats.capacity, 1);
    assert_eq!(value.pool.occupied(), 16);
    value.event(&stop(25, false), None, false);
    assert_eq!(value.stats.stopped, 1);
    assert_eq!(
        value.pool.occupied(),
        0,
        "pending sources are cancelled before submission"
    );
    let mut submitted = 0;
    value.flush(|_| {
        submitted += 1;
        true
    });
    assert_eq!(submitted, 0);
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
        include_bytes!("../../../crates/assets/data/block-physics-v2168.bin"),
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
