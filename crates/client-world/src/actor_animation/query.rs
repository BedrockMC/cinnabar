use super::{evaluation::MolangValue, *};

// Actor flag bits and metadata keys follow gophertunnel v1.61.0
// `minecraft/protocol/entity_metadata.go` (`EntityDataFlag*` and `EntityDataKey*`, iota from
// zero); flag bits from 64 live in the overflow flag word.
const FLAG_QUERIES: [(&str, u32); 56] = [
    ("blocking", FLAG_BLOCKING),
    ("can_damage_nearby_mobs", FLAG_DAMAGE_NEARBY_MOBS),
    ("facing_target_to_range_attack", 88),
    ("has_dash_cooldown", 108),
    ("is_admiring", 94),
    ("is_angry", 25),
    ("is_baby", FLAG_BABY),
    ("is_casting", 42),
    ("is_celebrating", 93),
    ("is_celebrating_special", 95),
    ("is_charged", 27),
    ("is_charging", 43),
    ("is_chested", 36),
    ("is_crawling", 114),
    ("is_croaking", 101),
    ("is_dancing", 51),
    ("is_delayed_attacking", 85),
    ("is_digging", 106),
    ("is_eating", 63),
    ("is_eating_mob", 102),
    ("is_elder", 33),
    ("is_emerging", 104),
    ("is_emoting", 92),
    ("is_gliding", FLAG_GLIDING),
    ("is_in_ui", 90),
    ("is_interested", 26),
    ("is_invisible", 5),
    ("is_jump_goal_jumping", 103),
    ("is_laying_egg", 60),
    ("is_leashed", 30),
    ("is_playing_dead", 98),
    ("is_powered", 9),
    ("is_pregnant", 59),
    ("is_resting", 23),
    ("is_roaring", 84),
    ("is_saddled", 8),
    ("is_scared", 68),
    ("is_searching", 113),
    ("is_shaking", 40),
    ("is_shaking_wetness", 40),
    ("is_sheared", 31),
    ("is_sitting", 24),
    ("is_sneaking", FLAG_SNEAKING),
    ("is_sniffing", 105),
    ("is_sonic_boom", 107),
    ("is_sprinting", 3),
    ("is_stalking", 91),
    ("is_standing", 39),
    ("is_stunned", 83),
    ("is_swimming", 57),
    ("is_tamed", 28),
    ("is_using_item", 4),
    ("show_bottom", 38),
    ("timer_flag_1", 115),
    ("timer_flag_2", 116),
    ("timer_flag_3", 117),
];
pub(super) const FLAG_SNEAKING: u32 = 1;
pub(super) const FLAG_BABY: u32 = 11;
pub(super) const FLAG_BLOCKING: u32 = 72;
pub(super) const FLAG_DAMAGE_NEARBY_MOBS: u32 = 56;
pub(super) const FLAG_GLIDING: u32 = 32;

const INTEGER_QUERIES: [(&str, u32); 10] = [
    ("fuse_time", 55),
    ("hurt_direction", 12),
    ("hurt_time", 11),
    ("invulnerable_ticks", 48),
    ("mark_variant", 43),
    ("skin_id", 104),
    ("structural_integrity", 1),
    ("swelling_dir", 21),
    ("trade_tier", 101),
    ("variant", 2),
];
const KEY_CARRY_BLOCK: u32 = 23;
const FLOAT_QUERIES: [(&str, u32, f32); 3] = [
    ("model_scale", 38, 1.0),
    ("sit_amount", 89, 0.0),
    ("lie_amount", 93, 0.0),
];
const KEY_NAME: u32 = 4;

// First-person and use-item queries the pack reads but the client has no timing, equipment, or
// game-mode source for yet. Each returns its vanilla idle value so the pre-animation formulas
// (item_use_normalized, helmet_layer_visible) and the use/crossbow animations stay neutral.
// Wiring the real sources later replaces the entry, not the query name.
const IDLE_QUERIES: [(&str, f32); 5] = [
    ("main_hand_item_use_duration", 0.0),
    ("main_hand_item_max_duration", 0.0),
    ("item_remaining_use_duration", 0.0),
    ("has_head_gear", 0.0),
    ("is_spectator", 0.0),
];

// Head-over-body yaw bound for look-at queries; needs independent measurement.
const TARGET_YAW_LIMIT: f32 = 85.0;

/// Actor state one query reads.
#[derive(Clone, Copy)]
pub(super) struct QueryInputs<'a> {
    pub(super) actor: &'a ActorSnapshot,
    pub(super) input: &'a ActorTickInput,
    pub(super) context: &'a ActorTickContext,
    pub(super) anim_tick: u64,
    pub(super) life_tick: u64,
    /// Whether all and any animations of the controller state being left have finished.
    pub(super) finished: (bool, bool),
}

/// Reads one query from retained actor state; a listed query the client has no data for
/// reads its idle value (0.0, or `''` for names).
pub(super) fn query(
    evaluator: &QueryInputs<'_>,
    identifier: &str,
    arguments: &[MolangValue],
) -> MolangValue {
    let name = identifier.strip_prefix("query.").unwrap_or(identifier);
    let text = |value: Option<&str>| MolangValue::String(Arc::from(value.unwrap_or("")));
    match name {
        "get_equipped_item_name" => {
            text(hand_item(evaluator.context, arguments.first()).map(item_name))
        }
        "get_name" => text(match evaluator.actor.metadata.get(&KEY_NAME) {
            Some(ActorMetadataValue::String(name)) => Some(name.as_ref()),
            _ => None,
        }),
        "owner_identifier" => text(None),
        _ => MolangValue::Number(number(evaluator, name, arguments)),
    }
}

fn number(evaluator: &QueryInputs<'_>, name: &str, arguments: &[MolangValue]) -> f32 {
    let (actor, input, context) = (evaluator.actor, evaluator.input, evaluator.context);
    let argument = |index: usize| arguments.get(index).map(MolangValue::number);
    if let Some((_, bit)) = FLAG_QUERIES.iter().find(|(query, _)| *query == name) {
        return truth(actor_flag(actor, *bit));
    }
    if let Some((_, key)) = INTEGER_QUERIES.iter().find(|(query, _)| *query == name) {
        return metadata_number(actor, *key).unwrap_or(0.0);
    }
    if let Some((_, key, idle)) = FLOAT_QUERIES.iter().find(|(query, ..)| *query == name) {
        return metadata_number(actor, *key).unwrap_or(*idle);
    }
    if let Some((_, idle)) = IDLE_QUERIES.iter().find(|(query, _)| *query == name) {
        return *idle;
    }
    match name {
        "anim_time" => evaluator.anim_tick as f32 * 0.05,
        "life_time" => evaluator.life_tick as f32 * 0.05,
        "delta_time" => 0.05,
        "modified_distance_moved" => input.distance_moved,
        "modified_move_speed" => input.move_speed,
        "walk_distance" => input.walk_distance,
        "ground_speed" => input.velocity[0].hypot(input.velocity[2]),
        "vertical_speed" => input.velocity[1],
        "position_delta" => argument(0)
            .filter(|axis| (0.0..3.0).contains(axis))
            .map_or(0.0, |axis| input.position_delta[axis as usize]),
        "movement_direction" => {
            let length = input
                .position_delta
                .iter()
                .map(|axis| axis * axis)
                .sum::<f32>();
            argument(0)
                .filter(|axis| (0.0..3.0).contains(axis) && length > 0.0)
                .map_or(0.0, |axis| {
                    input.position_delta[axis as usize] / length.sqrt()
                })
        }
        "is_carrying_block" => truth(metadata_number(actor, KEY_CARRY_BLOCK).unwrap_or(0.0) != 0.0),
        "is_on_ground" => truth(input.on_ground),
        "is_riding" => truth(input.is_riding),
        "is_moving" => truth(input.position_delta.iter().any(|axis| *axis != 0.0)),
        "is_alive" => truth(health(actor).is_none_or(|health| health > 0.0)),
        "health" => health(actor).unwrap_or(0.0),
        "is_sleeping" => truth(actor.player_is_sleeping()),
        "body_y_rotation" => input.body_yaw,
        "body_x_rotation" | "target_x_rotation" => input.pitch,
        "target_y_rotation" => head_relative_yaw(input, TARGET_YAW_LIMIT),
        // Only the one-argument forms carry a value; the bare forms read as zero.
        "head_y_rotation" => argument(0).map_or(0.0, |limit| head_relative_yaw(input, limit.abs())),
        "head_x_rotation" => argument(0).map_or(0.0, |_| input.pitch),
        "all_animations_finished" => truth(evaluator.finished.0),
        "any_animation_finished" => truth(evaluator.finished.1),
        "is_item_equipped" => truth(hand_item(context, arguments.first()).is_some()),
        "is_item_name_any" => truth(item_name_matches(context, arguments)),
        "is_riding_any_entity_of_type" => truth(context.ridden.as_deref().is_some_and(|ridden| {
            arguments
                .iter()
                .any(|name| matches!(name, MolangValue::String(name) if name.as_ref() == ridden))
        })),
        "has_rider" => truth(context.has_rider),
        "has_player_rider" => truth(context.has_player_rider),
        _ => 0.0,
    }
}

/// The equipped item a hand argument names: 0 or `'main_hand'` (the default), 1 or
/// `'off_hand'`.
fn hand_item<'a>(context: &'a ActorTickContext, hand: Option<&MolangValue>) -> Option<&'a str> {
    let off_hand = match hand {
        None => false,
        Some(MolangValue::Number(value)) => value.trunc() == 1.0,
        Some(MolangValue::String(name)) => name.as_ref() == "off_hand",
    };
    if off_hand {
        context.off_hand.as_deref()
    } else {
        context.main_hand.as_deref()
    }
}

/// Legacy item name without its namespace.
fn item_name(identifier: &str) -> &str {
    identifier
        .split_once(':')
        .map_or(identifier, |(_, name)| name)
}

fn item_name_matches(context: &ActorTickContext, arguments: &[MolangValue]) -> bool {
    let Some(MolangValue::String(slot)) = arguments.first() else {
        return false;
    };
    let item = match slot.as_ref() {
        "slot.weapon.mainhand" => context.main_hand.as_deref(),
        "slot.weapon.offhand" => context.off_hand.as_deref(),
        _ => None,
    };
    let names = match arguments.get(1) {
        Some(MolangValue::Number(_)) => &arguments[2..],
        _ => &arguments[1..],
    };
    item.is_some_and(|item| {
        names
            .iter()
            .any(|name| matches!(name, MolangValue::String(name) if name.as_ref() == item))
    })
}

fn health(actor: &ActorSnapshot) -> Option<f32> {
    actor
        .attributes
        .get("minecraft:health")
        .map(|health| health.current)
}

fn metadata_number(actor: &ActorSnapshot, key: u32) -> Option<f32> {
    match actor.metadata.get(&key)? {
        ActorMetadataValue::Byte(value) => Some(f32::from(*value)),
        ActorMetadataValue::Short(value) => Some(f32::from(*value)),
        ActorMetadataValue::Int(value) => Some(*value as f32),
        ActorMetadataValue::Long(value) => Some(*value as f32),
        ActorMetadataValue::Float(value) => Some(*value),
        _ => None,
    }
}

pub(super) fn actor_flag(actor: &ActorSnapshot, bit: u32) -> bool {
    actor.flag(bit)
}

fn head_relative_yaw(input: &ActorTickInput, limit: f32) -> f32 {
    wrap_degrees(input.head_yaw - input.body_yaw).clamp(-limit, limit)
}

pub(super) fn wrap_degrees(degrees: f32) -> f32 {
    (degrees + 180.0).rem_euclid(360.0) - 180.0
}

fn truth(value: bool) -> f32 {
    if value { 1.0 } else { 0.0 }
}
