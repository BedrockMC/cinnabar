//! Frame systems feeding the audio engine: packets, local motion, ambience, weather and output.

use std::collections::HashSet;

use bevy::prelude::{
    App, Local, Message, MessageReader, NonSendMut, Res, ResMut, Time, Update, Vec3,
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
const UI_CLICK: &str = "random.click";

/// A local interface sound request (button press, inventory click) by sound definition name.
#[derive(Debug, Clone, PartialEq, Message)]
pub(crate) struct UiSoundCue(pub &'static str);

impl UiSoundCue {
    pub(crate) const CLICK: Self = Self(UI_CLICK);
}

pub(crate) fn configure(app: &mut App) {
    app.init_resource::<AudioSettings>()
        .add_message::<UiSoundCue>()
        .add_systems(
            Update,
            (
                ingest_audio_events,
                drive_local_motion,
                drive_ambience,
                drive_weather_and_particles,
                pump_audio,
            )
                .chain()
                .after(crate::named_audio::drain_live_named_audio),
        );
}

fn block_lookup<'a>(
    collisions: Option<&'a PhysicsCollisionRegistries>,
    mode: assets::NetworkIdMode,
) -> impl Fn(u32) -> Option<String> + 'a {
    move |runtime_id| {
        collisions?
            .block_identifier(mode, runtime_id)
            .map(str::to_owned)
    }
}

fn identifier_at(
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

fn is_water(identifier: Option<&str>) -> bool {
    identifier.is_some_and(|name| WATER_IDENTIFIERS.contains(&name))
}

#[derive(Default)]
pub(super) struct IngestState {
    stream: u64,
    last_sequence: u64,
}

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
    let Some(stream) = world.stream.as_ref() else {
        messages.clear();
        if state.stream != 0 {
            state.stream = 0;
            engine.stop_all();
        }
        return;
    };
    let session = stream.actor_session_id();
    if state.stream != session {
        state.stream = session;
        state.last_sequence = 0;
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
            protocol::AudioEvent::Level(level) => engine
                .bank()
                .and_then(|bank| route::level_sound_request(bank.tables(), level, &lookup)),
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
    let prefix = stream.and(dimension_ambience(dimension));
    engine.set_loop(
        "dimension",
        prefix.and_then(|prefix| loop_spec(&format!("{prefix}.loop"))),
    );

    let dark = stream.is_some_and(|stream| {
        let (block, sky) = stream.light_level_at([eye.x, eye.y, eye.z]);
        block == 0 && sky == 0
    });
    let unit = engine.unit();
    if state.mood.tick(dark && !underwater, dt, unit) {
        let name = prefix.map_or_else(|| "ambient.cave".to_owned(), |p| format!("{p}.mood"));
        engine.enqueue(SoundRequest::new(name));
    }
    let unit = engine.unit();
    if let Some(prefix) = prefix
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
    world: Res<ClientWorld>,
    collisions: Option<Res<PhysicsCollisionRegistries>>,
    mix: Option<Res<PrecipitationMix>>,
    particles: Option<ResMut<ParticleSystem>>,
    inbox: Option<ResMut<ParticleInbox>>,
    mut engine: ResMut<AudioEngine>,
    mut seen_bolts: Local<HashSet<i64>>,
) {
    let mut particles = particles;
    let mut inbox = inbox;
    let Some(stream) = world.stream.as_ref() else {
        engine.set_loop("rain", None);
        seen_bolts.clear();
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
            engine.enqueue(SoundRequest::new("ambient.weather.lightning.impact").at(bolt.position));
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
    let mut requests = Vec::new();
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
