//! Frame systems feeding the audio engine: packets, local motion, ambience, weather and output.

use std::{
    collections::{HashMap, HashSet},
    sync::{
        Arc,
        atomic::{AtomicU32, Ordering},
    },
};

use bevy::prelude::{
    App, IntoScheduleConfigs, Local, Message, MessageReader, NonSendMut, Res, ResMut, Time, Update,
    Vec3,
};
use render::{ParticleSystem, PrecipitationMix};
use sim::PaletteWorld;

use super::{
    ambient::{
        ADDITIONS_INTERVAL, IntervalTimer, MOOD_INTERVAL, MusicScheduler, dimension_ambience,
        music_key,
    },
    engine::{AudioEngine, Listener, LoopSpec, SoundRequest},
    local::{LocalCue, LocalMotion, MotionSample},
    predicted::{LocalBlockCue, drive_actor_audio, drive_block_cues, drive_consume_audio},
    route,
    settings::{AudioCategory, AudioSettings},
};
use crate::{
    local_player::LocalViewPose,
    movement::{LocalPhysicsController, PhysicsCollisionRegistries},
    named_audio::AudioDevice,
    particles::ParticleInbox,
    runtime::{audio::SequencedAudioEvent, world::ClientWorld},
    ui_runtime::UiRuntime,
};

const PLAYER: &str = "minecraft:player";
const FEET_PROBE_BELOW: f64 = 0.2;
const WATER_IDENTIFIERS: [&str; 2] = ["minecraft:water", "minecraft:flowing_water"];
const THUNDER_GRACE_SECONDS: f32 = 0.3;
const UI_CLICK: &str = "random.click";
const AUDIO_STAGE: usize = render::RuntimeStage::Audio as usize;

static PENDING_UI_CLICKS: AtomicU32 = AtomicU32::new(0);

/// Requests the interface click sound from any code path (no ECS access needed); coalesced per frame.
pub(crate) fn ui_click() {
    let _ = PENDING_UI_CLICKS.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |count| {
        Some(count.saturating_add(1))
    });
}

/// A local interface sound request by sound definition name; ECS callers may send this instead of
/// calling [`ui_click`].
#[derive(Debug, Clone, PartialEq, Message)]
pub(crate) struct UiSoundCue(pub &'static str);

pub(crate) fn configure(app: &mut App) {
    app.init_resource::<AudioSettings>()
        .add_message::<UiSoundCue>()
        .add_message::<LocalBlockCue>()
        .add_systems(
            Update,
            (
                render::begin_stage_span::<AUDIO_STAGE>,
                ingest_audio_events,
                drive_local_motion,
                drive_ambience,
                drive_weather_and_particles,
                drive_block_cues,
                drive_consume_audio,
                drive_actor_audio,
                pump_audio,
                render::end_stage_span::<AUDIO_STAGE>,
            )
                .chain()
                .after(crate::named_audio::drain_live_named_audio),
        );
}

pub(super) fn block_lookup<'a>(
    collisions: Option<&'a PhysicsCollisionRegistries>,
    mode: assets::NetworkIdMode,
) -> impl Fn(u32) -> Option<String> + 'a {
    move |runtime_id| {
        collisions?
            .block_identifier(mode, runtime_id)
            .map(str::to_owned)
    }
}

pub(super) fn identifier_at(
    world: &PaletteWorld<'_>,
    collisions: &PhysicsCollisionRegistries,
    mode: assets::NetworkIdMode,
    block: [i32; 3],
) -> Option<String> {
    let runtime_id = world.primary_runtime_id(block).ok()?;
    collisions
        .block_identifier(mode, runtime_id)
        .map(str::to_owned)
}

pub(super) fn is_water(identifier: Option<&str>) -> bool {
    identifier.is_some_and(|name| WATER_IDENTIFIERS.contains(&name))
}

#[derive(Default)]
pub(super) struct IngestState {
    stream: u64,
    last_sequence: u64,
    /// Record voices by jukebox cell, so a stop event can silence the right one.
    records: HashMap<[i32; 3], Arc<str>>,
}

/// Level sound events also produced by local prediction; the second copy within the window is dropped.
const DEDUPED_EVENTS: [&str; 4] = ["place", "break", "hurt", "death"];
const RECORD_EVENT: i32 = 1006;

#[allow(clippy::too_many_arguments)]
pub(super) fn ingest_audio_events(
    mut messages: MessageReader<SequencedAudioEvent>,
    mut cues: MessageReader<UiSoundCue>,
    world: Res<ClientWorld>,
    collisions: Option<Res<PhysicsCollisionRegistries>>,
    mut engine: ResMut<AudioEngine>,
    mut state: Local<IngestState>,
) {
    for cue in cues.read() {
        engine.enqueue(SoundRequest::new(cue.0));
    }
    if PENDING_UI_CLICKS.swap(0, Ordering::Relaxed) > 0 {
        engine.enqueue(SoundRequest::new(UI_CLICK));
    }
    let Some(stream) = world.stream.as_ref() else {
        messages.clear();
        if state.stream != 0 {
            state.stream = 0;
            state.records.clear();
            engine.stop_all();
            engine.clear_server_if_current();
        }
        return;
    };
    let session = stream.actor_session_id();
    if state.stream != session {
        state.stream = session;
        state.last_sequence = 0;
        state.records.clear();
        engine.stop_all();
    }
    let dimension = stream.current_dimension();
    let lookup = block_lookup(collisions.as_deref(), stream.network_id_mode());
    for event in messages.read() {
        if event.origin_stream_session_id != session
            || event.dimension != dimension
            || event.sequence <= state.last_sequence
        {
            engine.stats.stale += 1;
            continue;
        }
        state.last_sequence = event.sequence;
        let request = match &event.event {
            protocol::AudioEvent::Play(play) => Some(route::play_request(play)),
            protocol::AudioEvent::Stop(stop) => {
                if stop.stop_all_sounds {
                    engine.stop_all();
                } else {
                    engine.stop_named(&stop.name);
                }
                continue;
            }
            protocol::AudioEvent::Level(level) => {
                let name = level.sound_event.as_ref();
                if DEDUPED_EVENTS.contains(&name) {
                    if engine.was_recent(name, level.position, 0.6, 3.0) {
                        continue;
                    }
                    engine.note_recent(name, level.position);
                } else if name == "thunder" {
                    engine.note_recent(name, level.position);
                }
                engine
                    .bank()
                    .and_then(|bank| route::level_sound_request(bank.tables(), level, &lookup))
            }
            protocol::AudioEvent::LevelEvent(level) if level.event_id == RECORD_EVENT => {
                let cell = level.position.map(|axis| axis.floor() as i32);
                if let Some(previous) = state.records.remove(&cell) {
                    engine.stop_named(&previous);
                }
                let name = (level.data != 0)
                    .then(|| stream.item_identifier(level.data))
                    .flatten()
                    .and_then(|identifier| route::record_sound_name(&identifier))
                    .filter(|name| {
                        engine
                            .bank()
                            .is_some_and(|bank| bank.definition(name).is_some())
                    });
                if let Some(name) = name {
                    let name: Arc<str> = name.into();
                    state.records.insert(cell, Arc::clone(&name));
                    engine.enqueue(SoundRequest::new(name).at(level.position));
                }
                continue;
            }
            protocol::AudioEvent::LevelEvent(level) => engine
                .bank()
                .and_then(|bank| route::level_event_request(bank.tables(), level)),
        };
        match request {
            Some(request) => engine.enqueue(request),
            None => engine.stats.unrouted += 1,
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn drive_local_motion(
    world: Res<ClientWorld>,
    collisions: Option<Res<PhysicsCollisionRegistries>>,
    physics: Res<LocalPhysicsController>,
    mut engine: ResMut<AudioEngine>,
    mut motion: Local<LocalMotion>,
    mut last_tick: Local<Option<u64>>,
) {
    let (Some(stream), Some(collisions), Some(state)) = (
        world.stream.as_ref(),
        collisions.as_deref(),
        physics.state(),
    ) else {
        motion.reset();
        *last_tick = None;
        return;
    };
    if *last_tick == Some(state.tick) || !engine.has_bank() {
        return;
    }
    *last_tick = Some(state.tick);
    let mode = stream.network_id_mode();
    let palette = PaletteWorld::new(
        stream.collision_store(),
        collisions.registry(mode),
        stream.current_dimension(),
    );
    let position = [state.position.x, state.position.y, state.position.z];
    let cell = |dy: f64| {
        [
            position[0].floor() as i32,
            (position[1] + dy).floor() as i32,
            position[2].floor() as i32,
        ]
    };
    let in_water = is_water(identifier_at(&palette, collisions, mode, cell(0.5)).as_deref());
    let below = identifier_at(&palette, collisions, mode, cell(-FEET_PROBE_BELOW));
    let sneaking = physics
        .latest_sneak_sprint()
        .is_some_and(|(sneak, _)| sneak);
    let cues = motion.advance(MotionSample {
        position,
        velocity_y: state.velocity.y,
        on_ground: state.on_ground,
        sneaking,
        in_water,
    });
    let feet = [position[0] as f32, position[1] as f32, position[2] as f32];
    let requests: Vec<SoundRequest> = {
        let Some(bank) = engine.bank() else { return };
        let tables = bank.tables();
        let material = below.as_deref().and_then(|name| tables.material_of(name));
        let interactive = |event: &str| {
            tables
                .interactive(PLAYER, event, material?)
                .map(|route| (route.sound, route.volume, route.pitch))
        };
        let entity = |event: &str| {
            tables
                .entity(PLAYER, event, None)
                .map(|route| (route.sound, route.volume, route.pitch))
        };
        cues.iter()
            .filter_map(|cue| match cue {
                LocalCue::Step => interactive("step"),
                LocalCue::Jump => interactive("jump"),
                LocalCue::Land { .. } => interactive("land"),
                LocalCue::Swim => entity("swim"),
                LocalCue::Splash => entity("splash"),
            })
            .map(|(sound, volume, pitch)| {
                SoundRequest::new(sound).with_ranges(volume, pitch).at(feet)
            })
            .collect()
    };
    for request in requests {
        engine.enqueue(request);
    }
}

pub(super) struct AmbientState {
    underwater: bool,
    mood: IntervalTimer,
    additions: IntervalTimer,
    music: MusicScheduler,
}

impl Default for AmbientState {
    fn default() -> Self {
        Self {
            underwater: false,
            mood: IntervalTimer::new(MOOD_INTERVAL),
            additions: IntervalTimer::new(ADDITIONS_INTERVAL),
            music: MusicScheduler::default(),
        }
    }
}

/// Ambience definition prefix for the eye position: the biome's own set when the pack defines it.
fn ambience_prefix(
    stream: &client_world::WorldStream,
    dimension: i32,
    eye: [f32; 3],
    engine: &AudioEngine,
) -> Option<String> {
    let fallback = dimension_ambience(dimension)?;
    let biome = stream.camera_biome_id(eye).and_then(|id| {
        stream
            .biome_definitions_snapshot()
            .iter()
            .find(|definition| u32::from(definition.biome_id.unwrap_or(u16::MAX)) == id)
            .map(|definition| definition.name.to_string())
    });
    let own = biome
        .map(|name| format!("ambient.{name}"))
        .filter(|prefix| {
            engine
                .bank()
                .is_some_and(|bank| bank.definition(&format!("{prefix}.loop")).is_some())
        });
    Some(own.unwrap_or_else(|| fallback.to_owned()))
}

#[allow(clippy::too_many_arguments)]
pub(super) fn drive_ambience(
    time: Res<Time>,
    world: Res<ClientWorld>,
    collisions: Option<Res<PhysicsCollisionRegistries>>,
    view: Res<LocalViewPose>,
    ui: Option<Res<UiRuntime>>,
    mut engine: ResMut<AudioEngine>,
    mut state: Local<AmbientState>,
) {
    if !engine.has_bank() {
        return;
    }
    let dt = time.delta_secs();
    let stream = world.stream.as_ref();
    let dimension = stream.map_or(0, |stream| stream.current_dimension());
    let creative = ui
        .as_deref()
        .and_then(UiRuntime::player_game_mode)
        .is_some_and(|mode| matches!(mode, protocol::PlayerGameMode::Creative));

    let eye = view.eye_translation();
    let underwater = match (stream, collisions.as_deref()) {
        (Some(stream), Some(collisions)) => {
            let mode = stream.network_id_mode();
            let palette = PaletteWorld::new(
                stream.collision_store(),
                collisions.registry(mode),
                stream.current_dimension(),
            );
            let block = [
                eye.x.floor() as i32,
                eye.y.floor() as i32,
                eye.z.floor() as i32,
            ];
            is_water(identifier_at(&palette, collisions, mode, block).as_deref())
        }
        _ => false,
    };
    if underwater != state.underwater {
        let name = if underwater {
            "ambient.underwater.enter"
        } else {
            "ambient.underwater.exit"
        };
        engine.enqueue(SoundRequest::new(name));
        state.underwater = underwater;
    }
    let loop_spec = |name: &str| {
        Some(LoopSpec {
            name: name.into(),
            volume: 1.0,
        })
    };
    engine.set_loop(
        "underwater",
        underwater
            .then(|| loop_spec("ambient.underwater.loop"))
            .flatten(),
    );
    let prefix: Option<String> = stream
        .and_then(|stream| ambience_prefix(stream, dimension, [eye.x, eye.y, eye.z], &engine));
    engine.set_loop(
        "dimension",
        prefix
            .as_deref()
            .and_then(|prefix| loop_spec(&format!("{prefix}.loop"))),
    );

    // Unloaded chunks read as unlit, so mood waits for the eye's chunk to be present.
    let dark = stream.is_some_and(|stream| {
        let at = [eye.x, eye.y, eye.z];
        let (block, sky) = stream.light_level_at(at);
        stream.camera_biome_id(at).is_some() && block == 0 && sky == 0
    });
    let unit = engine.unit();
    if state.mood.tick(dark && !underwater, dt, unit) {
        let name = prefix
            .as_deref()
            .map_or_else(|| "ambient.cave".to_owned(), |p| format!("{p}.mood"));
        engine.enqueue(SoundRequest::new(name));
    }
    let unit = engine.unit();
    if let Some(prefix) = prefix.as_deref()
        && state.additions.tick(true, dt, unit)
    {
        engine.enqueue(SoundRequest::new(format!("{prefix}.additions")));
    }

    let key = music_key(stream.is_some(), dimension, creative);
    let entry = engine
        .bank()
        .and_then(|bank| bank.music(key))
        .map(|entry| (entry.event_name.clone(), (entry.min_delay, entry.max_delay)));
    if let Some((event_name, delay)) = entry {
        let playing = engine.is_playing_category(AudioCategory::Music);
        let mut rolls = [engine.unit(), engine.unit()].into_iter();
        let start = state
            .music
            .update(key, delay, playing, dt, || rolls.next().unwrap_or(0.5));
        if start {
            engine.enqueue(SoundRequest::new(&*event_name));
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn drive_weather_and_particles(
    time: Res<Time>,
    world: Res<ClientWorld>,
    collisions: Option<Res<PhysicsCollisionRegistries>>,
    mix: Option<Res<PrecipitationMix>>,
    particles: Option<ResMut<ParticleSystem>>,
    inbox: Option<ResMut<ParticleInbox>>,
    mut engine: ResMut<AudioEngine>,
    mut seen_bolts: Local<HashSet<i64>>,
    mut pending_bolts: Local<Vec<(f32, [f32; 3])>>,
) {
    let mut particles = particles;
    let mut inbox = inbox;
    let Some(stream) = world.stream.as_ref() else {
        engine.set_loop("rain", None);
        seen_bolts.clear();
        pending_bolts.clear();
        return;
    };
    if !engine.has_bank() {
        return;
    }
    let rain = mix.as_deref().map_or(0.0, |mix| mix.rain.clamp(0.0, 1.0));
    engine.set_loop(
        "rain",
        (rain > 0.01).then(|| LoopSpec {
            name: "ambient.weather.rain".into(),
            volume: rain,
        }),
    );

    let bolts = stream.lightning_bolts();
    seen_bolts.retain(|id| bolts.iter().any(|bolt| bolt.unique_id == *id));
    for bolt in &bolts {
        if seen_bolts.insert(bolt.unique_id) {
            pending_bolts.push((THUNDER_GRACE_SECONDS, bolt.position));
        }
    }
    // The server usually voices bolts itself; only unvoiced ones get the local fallback.
    let dt = time.delta_secs();
    let mut due = Vec::new();
    pending_bolts.retain_mut(|(remaining, position)| {
        *remaining -= dt;
        if *remaining <= 0.0 {
            due.push(*position);
        }
        *remaining > 0.0
    });
    for position in due {
        if !engine.was_recent("thunder", position, 2.0, f32::MAX.sqrt()) {
            engine.enqueue(SoundRequest::new("ambient.weather.lightning.impact").at(position));
            engine.enqueue(SoundRequest::new("ambient.weather.thunder"));
        }
    }

    let sounds = particles
        .as_mut()
        .map(|system| system.take_sounds())
        .unwrap_or_default();
    let destroyed = inbox
        .as_mut()
        .map(|inbox| inbox.take_level_audio())
        .unwrap_or_default();
    let lookup = block_lookup(collisions.as_deref(), stream.network_id_mode());
    let mut requests: Vec<SoundRequest> = Vec::new();
    if let Some(bank) = engine.bank() {
        let tables = bank.tables();
        for sound in &sounds {
            let request = match tables.individual(&sound.name) {
                Some(route) => {
                    SoundRequest::new(route.sound.clone()).with_ranges(route.volume, route.pitch)
                }
                None => SoundRequest::new(&*sound.name),
            };
            requests.push(request.at(sound.position));
        }
        for (id, position, data) in destroyed {
            requests.extend(route::destroy_block_request(
                tables, id, position, data, &lookup,
            ));
        }
    }
    for request in requests {
        engine.enqueue(request);
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn pump_audio(
    time: Res<Time>,
    view: Res<LocalViewPose>,
    settings: Res<AudioSettings>,
    mut engine: ResMut<AudioEngine>,
    mut device: Option<NonSendMut<AudioDevice>>,
) {
    engine.poll_server();
    let eye = view.eye_translation();
    let right = view.rotation() * Vec3::X;
    let listener = Listener {
        position: [eye.x, eye.y, eye.z],
        right: [right.x, right.y, right.z],
    };
    let sources = engine.pump(Some(listener), time.delta_secs(), &settings);
    let Some(device) = device.as_mut() else {
        return;
    };
    for source in sources {
        if !device.play_source(source) {
            engine.stats.backend_failed += 1;
            break;
        }
    }
}
