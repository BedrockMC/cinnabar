use super::{evaluation::bool_value, *};

// Actor flag bit numbers follow gophertunnel v1.61.0
// `minecraft/protocol/entity_metadata.go` (`EntityDataFlag*`, iota from zero); bits from 64
// live in the overflow flag word.
const FLAG_SNEAKING: u32 = 1;
const FLAG_SPRINTING: u32 = 3;
pub(super) const FLAG_BABY: u32 = 11;
const FLAG_RESTING: u32 = 23;
const FLAG_SITTING: u32 = 24;
const FLAG_ANGRY: u32 = 25;
const FLAG_INTERESTED: u32 = 26;
const FLAG_CHARGED: u32 = 27;
const FLAG_TAMED: u32 = 28;
const FLAG_LEASHED: u32 = 30;
const FLAG_GLIDING: u32 = 32;
const FLAG_SHAKING: u32 = 40;
const FLAG_CASTING: u32 = 42;
const FLAG_CHARGING: u32 = 43;
const FLAG_DANCING: u32 = 51;
const FLAG_SWIMMING: u32 = 57;
const FLAG_PREGNANT: u32 = 59;
const FLAG_LAYING_EGG: u32 = 60;
const FLAG_EATING: u32 = 63;
const FLAG_SCARED: u32 = 68;
const FLAG_BLOCKING: u32 = 72;
const FLAG_SLEEPING: u32 = 76;
const FLAG_STUNNED: u32 = 83;
const FLAG_ROARING: u32 = 84;
const FLAG_DELAYED_ATTACK: u32 = 85;
const FLAG_FACING_TARGET_TO_RANGE_ATTACK: u32 = 88;
const FLAG_STALKING: u32 = 91;
const FLAG_EMOTING: u32 = 92;
const FLAG_CELEBRATING: u32 = 93;
const FLAG_ADMIRING: u32 = 94;
const FLAG_PLAYING_DEAD: u32 = 98;
const FLAG_CROAKING: u32 = 101;
const FLAG_DIGEST_MOB: u32 = 102;
const FLAG_JUMP_GOAL: u32 = 103;
const FLAG_SNIFFING: u32 = 105;
const FLAG_DIGGING: u32 = 106;
const FLAG_SONIC_BOOM: u32 = 107;
const FLAG_HAS_DASH_TIMEOUT: u32 = 108;
const FLAG_SEARCHING: u32 = 113;
const FLAG_CRAWLING: u32 = 114;
const FLAG_TIMER_1: u32 = 115;

const PLAYER_FLAGS_METADATA_KEY: u32 = 26;
const PLAYER_FLAGS_SLEEPING: u8 = 1 << 1;
const FLAGS_EXTENDED_METADATA_KEY: u32 = 92;

/// Tick clocks a query reads, relative to the animation reset and the actor spawn.
#[derive(Clone, Copy)]
pub(super) struct QueryClock {
    pub(super) anim_tick: u64,
    pub(super) life_tick: u64,
}

/// Reads one reviewed query from retained actor state; untracked state reads as idle zero.
pub(super) fn query(
    actor: &ActorSnapshot,
    input: &ActorTickInput,
    clock: QueryClock,
    identifier: &str,
    argument: Option<f32>,
) -> f32 {
    {
        let flag = |bit| bool_value(actor_flag(actor, bit));
        match identifier.strip_prefix("query.").unwrap_or(identifier) {
            "anim_time" => clock.anim_tick as f32 * 0.05,
            "life_time" => clock.life_tick as f32 * 0.05,
            "modified_distance_moved" => input.distance_moved,
            "modified_move_speed" => input.move_speed,
            "ground_speed" => input.velocity[0].hypot(input.velocity[2]),
            "vertical_speed" => input.velocity[1],
            "position_delta" => argument
                .filter(|axis| (0.0..3.0).contains(axis))
                .map_or(0.0, |axis| input.position_delta[axis as usize]),
            "is_on_ground" => bool_value(input.on_ground),
            "is_riding" => bool_value(input.is_riding),
            "is_moving" => bool_value(input.position_delta.iter().any(|axis| *axis != 0.0)),
            "is_alive" => bool_value(
                actor
                    .attributes
                    .get("minecraft:health")
                    .is_none_or(|health| health.current > 0.0),
            ),
            "is_sleeping" => {
                bool_value(player_sleeping_flag(actor) || actor_flag(actor, FLAG_SLEEPING))
            }
            "body_y_rotation" => input.body_yaw,
            "target_x_rotation" => input.pitch,
            "target_y_rotation" => head_relative_yaw(input, TARGET_YAW_LIMIT),
            // Only the one-argument forms carry a value; the bare forms read as zero.
            "head_y_rotation" => {
                argument.map_or(0.0, |limit| head_relative_yaw(input, limit.abs()))
            }
            "head_x_rotation" => argument.map_or(0.0, |_| input.pitch),
            "is_sneaking" => flag(FLAG_SNEAKING),
            "is_sprinting" => flag(FLAG_SPRINTING),
            "is_baby" => flag(FLAG_BABY),
            "is_resting" => flag(FLAG_RESTING),
            "is_sitting" => flag(FLAG_SITTING),
            "is_angry" => flag(FLAG_ANGRY),
            "is_interested" => flag(FLAG_INTERESTED),
            "is_charged" => flag(FLAG_CHARGED),
            "is_tamed" => flag(FLAG_TAMED),
            "is_leashed" => flag(FLAG_LEASHED),
            "is_gliding" => flag(FLAG_GLIDING),
            "is_shaking_wetness" => flag(FLAG_SHAKING),
            "is_casting" => flag(FLAG_CASTING),
            "is_charging" => flag(FLAG_CHARGING),
            "is_dancing" => flag(FLAG_DANCING),
            "is_swimming" => flag(FLAG_SWIMMING),
            "is_pregnant" => flag(FLAG_PREGNANT),
            "is_laying_egg" => flag(FLAG_LAYING_EGG),
            "is_eating" => flag(FLAG_EATING),
            "is_scared" => flag(FLAG_SCARED),
            "blocking" => flag(FLAG_BLOCKING),
            "is_stunned" => flag(FLAG_STUNNED),
            "is_roaring" => flag(FLAG_ROARING),
            "is_delayed_attacking" => flag(FLAG_DELAYED_ATTACK),
            "facing_target_to_range_attack" => flag(FLAG_FACING_TARGET_TO_RANGE_ATTACK),
            "is_stalking" => flag(FLAG_STALKING),
            "is_emoting" => flag(FLAG_EMOTING),
            "is_celebrating" => flag(FLAG_CELEBRATING),
            "is_admiring" => flag(FLAG_ADMIRING),
            "is_playing_dead" => flag(FLAG_PLAYING_DEAD),
            "is_croaking" => flag(FLAG_CROAKING),
            "is_eating_mob" => flag(FLAG_DIGEST_MOB),
            "is_jump_goal_jumping" => flag(FLAG_JUMP_GOAL),
            "is_sniffing" => flag(FLAG_SNIFFING),
            "is_digging" => flag(FLAG_DIGGING),
            "is_sonic_boom" => flag(FLAG_SONIC_BOOM),
            "has_dash_cooldown" => flag(FLAG_HAS_DASH_TIMEOUT),
            "is_searching" => flag(FLAG_SEARCHING),
            "is_crawling" => flag(FLAG_CRAWLING),
            "timer_flag_1" => flag(FLAG_TIMER_1),
            "timer_flag_2" => flag(FLAG_TIMER_1 + 1),
            "timer_flag_3" => flag(FLAG_TIMER_1 + 2),
            _ => 0.0,
        }
    }
}

pub(super) fn actor_flag(actor: &ActorSnapshot, bit: u32) -> bool {
    let (key, bit) = if bit < 64 {
        (0, bit)
    } else {
        (FLAGS_EXTENDED_METADATA_KEY, bit - 64)
    };
    match actor.metadata.get(&key) {
        Some(ActorMetadataValue::Flags(flags) | ActorMetadataValue::FlagsExtended(flags)) => {
            flags & (1_u64 << bit) != 0
        }
        _ => false,
    }
}

fn player_sleeping_flag(actor: &ActorSnapshot) -> bool {
    matches!(
        actor.metadata.get(&PLAYER_FLAGS_METADATA_KEY),
        Some(ActorMetadataValue::Byte(flags)) if (*flags as u8) & PLAYER_FLAGS_SLEEPING != 0
    )
}

// Head-over-body yaw bound for look-at queries; needs independent measurement.
const TARGET_YAW_LIMIT: f32 = 85.0;

fn head_relative_yaw(input: &ActorTickInput, limit: f32) -> f32 {
    wrap_degrees(input.head_yaw - input.body_yaw).clamp(-limit, limit)
}

pub(super) fn wrap_degrees(degrees: f32) -> f32 {
    (degrees + 180.0).rem_euclid(360.0) - 180.0
}
