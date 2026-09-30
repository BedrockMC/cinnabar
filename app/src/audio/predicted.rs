//! Client-predicted sounds tied to local interaction: block place/hit/break, eating and drinking,
//! the local player's hurt/death, and dropped-item pickup.

use std::{collections::HashSet, sync::Arc};

use bevy::prelude::{Local, Message, MessageReader, Res, ResMut, Time};
use protocol::ActorStatusKind;
use sim::PaletteWorld;

use super::{
    engine::{AudioEngine, SoundRequest},
    systems::{block_lookup, identifier_at},
};
use crate::{
    local_player::LocalViewPose, movement::PhysicsCollisionRegistries, particles::ParticleInbox,
    runtime::world::ClientWorld, semantic_controls::SemanticInputSnapshot,
    survival_mining::SurvivalMiningRuntime, ui_runtime::UiRuntime,
};

const PLAYER: &str = "minecraft:player";
/// Seconds between block hit sounds while mining; needs native measurement.
const HIT_INTERVAL: f32 = 0.25;
/// Seconds between eating/drinking sounds while an item is in use; needs native measurement.
const CONSUME_INTERVAL: f32 = 0.25;
/// Held seconds after which releasing a food item counts as finishing it; needs native measurement.
const EAT_DURATION: f32 = 1.6;
const DEDUPE_SECONDS: f64 = 0.6;
const DEDUPE_RADIUS: f32 = 3.0;

const DRINKS: [&str; 5] = [
    "minecraft:potion",
    "minecraft:milk_bucket",
    "minecraft:honey_bottle",
    "minecraft:ominous_bottle",
    "minecraft:experience_bottle",
];
const FOODS: &[&str] = &[
    "minecraft:apple",
    "minecraft:golden_apple",
    "minecraft:enchanted_golden_apple",
    "minecraft:baked_potato",
    "minecraft:potato",
    "minecraft:poisonous_potato",
    "minecraft:carrot",
    "minecraft:golden_carrot",
    "minecraft:beetroot",
    "minecraft:beetroot_soup",
    "minecraft:bread",
    "minecraft:cookie",
    "minecraft:melon_slice",
    "minecraft:pumpkin_pie",
    "minecraft:mushroom_stew",
    "minecraft:rabbit_stew",
    "minecraft:suspicious_stew",
    "minecraft:beef",
    "minecraft:cooked_beef",
    "minecraft:porkchop",
    "minecraft:cooked_porkchop",
    "minecraft:chicken",
    "minecraft:cooked_chicken",
    "minecraft:mutton",
    "minecraft:cooked_mutton",
    "minecraft:rabbit",
    "minecraft:cooked_rabbit",
    "minecraft:cod",
    "minecraft:cooked_cod",
    "minecraft:salmon",
    "minecraft:cooked_salmon",
    "minecraft:tropical_fish",
    "minecraft:pufferfish",
    "minecraft:rotten_flesh",
    "minecraft:spider_eye",
    "minecraft:dried_kelp",
    "minecraft:sweet_berries",
    "minecraft:glow_berries",
    "minecraft:chorus_fruit",
];

/// A local block interaction the audio runtime should voice before the server confirms it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Message)]
pub(crate) enum LocalBlockCue {
    /// A block item was placed at `position`; the id is the item's block runtime id.
    Place {
        position: [i32; 3],
        block_runtime_id: i32,
    },
}

#[derive(Debug, Default)]
pub(super) struct MiningAudio {
    target: Option<([i32; 3], Option<String>)>,
    hit_timer: f32,
}

fn center(cell: [i32; 3]) -> [f32; 3] {
    cell.map(|axis| axis as f32 + 0.5)
}

pub(super) fn is_consumable(identifier: &str) -> Option<&'static str> {
    if DRINKS.contains(&identifier) {
        Some("drink")
    } else if FOODS.contains(&identifier) {
        Some("eat")
    } else {
        None
    }
}

/// Voices local block placement, mining hits and the predicted break of the mined block.
#[allow(clippy::too_many_arguments)]
pub(super) fn drive_block_cues(
    mut cues: MessageReader<LocalBlockCue>,
    time: Res<Time>,
    world: Res<ClientWorld>,
    collisions: Option<Res<PhysicsCollisionRegistries>>,
    survival: Option<Res<SurvivalMiningRuntime>>,
    mut engine: ResMut<AudioEngine>,
    mut mining: Local<MiningAudio>,
) {
    let (Some(stream), Some(collisions)) = (world.stream.as_ref(), collisions.as_deref()) else {
        cues.clear();
        *mining = MiningAudio::default();
        return;
    };
    if !engine.has_bank() {
        cues.clear();
        return;
    }
    let mode = stream.network_id_mode();
    let lookup = block_lookup(Some(collisions), mode);
    let palette = PaletteWorld::new(
        stream.collision_store(),
        collisions.registry(mode),
        stream.current_dimension(),
    );
    let mut requests: Vec<(&'static str, Option<String>, [i32; 3])> = Vec::new();
    for cue in cues.read() {
        let LocalBlockCue::Place {
            position,
            block_runtime_id,
        } = *cue;
        requests.push(("place", lookup(block_runtime_id as u32), position));
    }
    let target = survival
        .as_deref()
        .and_then(SurvivalMiningRuntime::destroying_target)
        .map(|(cell, _face)| cell);
    let previous = mining.target.as_ref().map(|(cell, _)| *cell);
    if target != previous {
        if let Some((cell, identifier)) = mining.target.take()
            && palette.is_air(cell).unwrap_or(false)
        {
            requests.push(("break", identifier, cell));
        }
        mining.hit_timer = 0.0;
        mining.target = target.map(|cell| (cell, identifier_at(&palette, collisions, mode, cell)));
    }
    if let Some(cell) = mining.target.as_ref().map(|(cell, _)| *cell) {
        mining.hit_timer -= time.delta_secs();
        if mining.hit_timer <= 0.0 {
            mining.hit_timer = HIT_INTERVAL;
            let identifier = mining.target.as_ref().and_then(|(_, id)| id.clone());
            requests.push(("hit", identifier, cell));
        }
    }
    let built: Vec<(&'static str, [i32; 3], SoundRequest)> = {
        let Some(bank) = engine.bank() else { return };
        let tables = bank.tables();
        requests
            .into_iter()
            .filter_map(|(event, identifier, cell)| {
                let material = tables.material_of(identifier.as_deref()?)?;
                let route = tables.block(material, event)?;
                let request = SoundRequest::new(route.sound)
                    .with_ranges(route.volume, route.pitch)
                    .at(center(cell));
                Some((event, cell, request))
            })
            .collect()
    };
    for (event, cell, request) in built {
        let position = center(cell);
        if event != "hit" {
            if engine.was_recent(event, position, DEDUPE_SECONDS, DEDUPE_RADIUS) {
                continue;
            }
            engine.note_recent(event, position);
        }
        engine.enqueue(request);
    }
}

#[derive(Debug, Default)]
pub(super) struct ConsumeAudio {
    item: Option<Arc<str>>,
    elapsed: f32,
    timer: f32,
}

/// Loops the eating/drinking sound while a consumable is held in use, with a finishing burp.
#[allow(clippy::too_many_arguments)]
pub(super) fn drive_consume_audio(
    time: Res<Time>,
    input: Res<SemanticInputSnapshot>,
    ui: Option<Res<UiRuntime>>,
    world: Res<ClientWorld>,
    view: Res<LocalViewPose>,
    mut engine: ResMut<AudioEngine>,
    mut state: Local<ConsumeAudio>,
) {
    let held = input.phase(semantic_input::Action::Use).held;
    let selected = world
        .stream
        .as_ref()
        .zip(ui.as_deref())
        .and_then(|(stream, ui)| {
            stream
                .canonical_item_stack(ui.selected_stack()?)?
                .identifier
        });
    let kind = selected.as_deref().and_then(is_consumable);
    if !held || kind.is_none() || state.item != selected {
        let finished = state.elapsed >= EAT_DURATION
            && state.item.as_deref().and_then(is_consumable) == Some("eat");
        if finished && !held {
            engine.enqueue(SoundRequest::new("random.burp"));
        }
        state.elapsed = 0.0;
        state.timer = 0.0;
        state.item = selected;
        if !held || kind.is_none() {
            return;
        }
    }
    let Some(kind) = kind else { return };
    let dt = time.delta_secs();
    state.elapsed += dt;
    state.timer -= dt;
    if state.timer > 0.0 || !engine.has_bank() {
        return;
    }
    state.timer = CONSUME_INTERVAL;
    let eye = view.eye_translation();
    let request = engine.bank().and_then(|bank| {
        let route = bank.tables().entity(PLAYER, kind, None)?;
        Some(
            SoundRequest::new(route.sound)
                .with_ranges(route.volume, route.pitch)
                .at([eye.x, eye.y, eye.z]),
        )
    });
    if let Some(request) = request {
        engine.enqueue(request);
    }
}

/// Voices the local player's hurt/death status and dropped-item pickups.
pub(super) fn drive_actor_audio(
    world: Res<ClientWorld>,
    inbox: Option<ResMut<ParticleInbox>>,
    mut engine: ResMut<AudioEngine>,
    mut popped: Local<HashSet<u64>>,
) {
    let Some(stream) = world.stream.as_ref() else {
        popped.clear();
        if let Some(mut inbox) = inbox {
            inbox.take_status_audio();
        }
        return;
    };
    let notices = inbox
        .map(|mut inbox| inbox.take_status_audio())
        .unwrap_or_default();
    if !engine.has_bank() {
        return;
    }
    let local = stream.local_player_runtime_id();
    for notice in notices.iter().filter(|notice| notice.runtime_id == local) {
        let event = match notice.kind {
            ActorStatusKind::Hurt | ActorStatusKind::HurtWithoutDamage => "hurt",
            ActorStatusKind::Death => "death",
            _ => continue,
        };
        if engine.was_recent(event, notice.position, 0.4, DEDUPE_RADIUS) {
            continue;
        }
        engine.note_recent(event, notice.position);
        let request = engine.bank().and_then(|bank| {
            let route = bank.tables().entity(PLAYER, event, None)?;
            Some(
                SoundRequest::new(route.sound)
                    .with_ranges(route.volume, route.pitch)
                    .at(notice.position),
            )
        });
        if let Some(request) = request {
            engine.enqueue(request);
        }
    }
    let items = stream.dropped_items(0.0);
    popped.retain(|id| items.iter().any(|item| item.runtime_id == *id));
    for item in &items {
        let collected = stream
            .actor(item.runtime_id)
            .is_some_and(|actor| actor.status.pickup.is_some());
        if collected && popped.insert(item.runtime_id) {
            let request = engine
                .bank()
                .and_then(|bank| bank.tables().individual("pop").cloned())
                .map_or_else(
                    || SoundRequest::new("random.pop"),
                    |route| SoundRequest::new(route.sound).with_ranges(route.volume, route.pitch),
                );
            engine.enqueue(request.at(item.position));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn consumables_are_classified() {
        assert_eq!(is_consumable("minecraft:bread"), Some("eat"));
        assert_eq!(is_consumable("minecraft:potion"), Some("drink"));
        assert_eq!(is_consumable("minecraft:stone"), None);
    }

    #[test]
    fn cell_centers_offset_by_half_a_block() {
        assert_eq!(center([1, -2, 3]), [1.5, -1.5, 3.5]);
    }
}
