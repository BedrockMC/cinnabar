//! What a game mode lets the local player do, decoupled from the block-breaking
//! wire mode. Mode defaults follow dragonfly's `world.GameMode` interface (MIT);
//! server ability bits refine them where the session actually sent evidence.

use protocol::{AbilitiesUpdate, AbilityLayersEvidence, PlayerGameMode};

/// Documented survival/adventure melee reach from the eye.
const SURVIVAL_ATTACK_REACH: f64 = 3.0;
/// Creative melee reach. Needs independent measurement.
const CREATIVE_ATTACK_REACH: f64 = 7.0;

/// Bedrock ability bit positions, per the pinned gophertunnel
/// `minecraft/protocol/ability.go`. Only the bits the client gates on are named.
mod ability_bit {
    pub(super) const BUILD: u32 = 1 << 0;
    pub(super) const MINE: u32 = 1 << 1;
    pub(super) const INVULNERABLE: u32 = 1 << 8;
    pub(super) const FLYING: u32 = 1 << 9;
    pub(super) const MAY_FLY: u32 = 1 << 10;
    pub(super) const INSTANT_BUILD: u32 = 1 << 11;
    pub(super) const NO_CLIP: u32 = 1 << 17;
}

/// The interaction gates one game mode grants, after server ability overrides.
///
/// `attack_reach` is 0.0 exactly when attacking is disallowed. `creative_reach`
/// selects the block-pick reach profile, not a capability. The full surface is
/// modeled from the authoritative reference; not every field gates a producer yet.
#[derive(Debug, Clone, Copy, PartialEq)]
#[allow(dead_code)]
pub(crate) struct GameModeCapabilities {
    pub(crate) can_edit: bool,
    pub(crate) can_interact: bool,
    pub(crate) can_attack: bool,
    pub(crate) can_fly: bool,
    pub(crate) flying: bool,
    pub(crate) creative_inventory: bool,
    pub(crate) instant_break: bool,
    pub(crate) invulnerable: bool,
    pub(crate) visible: bool,
    pub(crate) has_collision: bool,
    pub(crate) attack_reach: f64,
    pub(crate) creative_reach: bool,
}

impl GameModeCapabilities {
    /// Mode defaults with no server evidence applied.
    pub(crate) const fn for_mode(mode: PlayerGameMode) -> Self {
        match mode {
            PlayerGameMode::Survival => Self {
                can_edit: true,
                can_interact: true,
                can_attack: true,
                can_fly: false,
                flying: false,
                creative_inventory: false,
                instant_break: false,
                invulnerable: false,
                visible: true,
                has_collision: true,
                attack_reach: SURVIVAL_ATTACK_REACH,
                creative_reach: false,
            },
            PlayerGameMode::Creative => Self {
                can_edit: true,
                can_interact: true,
                can_attack: true,
                can_fly: true,
                flying: false,
                creative_inventory: true,
                instant_break: true,
                invulnerable: true,
                visible: true,
                has_collision: true,
                attack_reach: CREATIVE_ATTACK_REACH,
                creative_reach: true,
            },
            // Adventure interacts and attacks; world editing waits for an
            // explicit server Build/Mine grant.
            PlayerGameMode::Adventure => Self {
                can_edit: false,
                can_interact: true,
                can_attack: true,
                can_fly: false,
                flying: false,
                creative_inventory: false,
                instant_break: false,
                invulnerable: false,
                visible: true,
                has_collision: true,
                attack_reach: SURVIVAL_ATTACK_REACH,
                creative_reach: false,
            },
            PlayerGameMode::Spectator => Self {
                can_edit: false,
                can_interact: false,
                can_attack: false,
                can_fly: true,
                flying: true,
                creative_inventory: false,
                instant_break: false,
                invulnerable: true,
                visible: false,
                has_collision: false,
                attack_reach: 0.0,
                creative_reach: false,
            },
            // Fail closed on an unresolved mode: no interaction until one arrives.
            PlayerGameMode::Unknown => Self {
                can_edit: false,
                can_interact: false,
                can_attack: false,
                can_fly: false,
                flying: false,
                creative_inventory: false,
                instant_break: false,
                invulnerable: false,
                visible: true,
                has_collision: true,
                attack_reach: 0.0,
                creative_reach: false,
            },
        }
    }

    /// Mode defaults with any server-sent ability bits folded in. Only bits an
    /// ability layer actually defines override a default; the rest stand.
    pub(crate) fn resolve(mode: PlayerGameMode, abilities: Option<&AbilitiesUpdate>) -> Self {
        let mut caps = Self::for_mode(mode);
        let Some(abilities) = abilities else {
            return caps;
        };
        let resolved = |bit: u32| resolved_ability(abilities, bit);
        // Editing follows Build/Mine when either is present: adventure gains
        // world editing only on an explicit grant.
        let build = resolved(ability_bit::BUILD);
        let mine = resolved(ability_bit::MINE);
        if build.is_some() || mine.is_some() {
            caps.can_edit = build.unwrap_or(false) || mine.unwrap_or(false);
        }
        if let Some(may_fly) = resolved(ability_bit::MAY_FLY) {
            caps.can_fly = may_fly;
        }
        if let Some(flying) = resolved(ability_bit::FLYING) {
            caps.flying = flying;
        }
        if let Some(instant) = resolved(ability_bit::INSTANT_BUILD) {
            caps.instant_break = instant;
        }
        if let Some(invulnerable) = resolved(ability_bit::INVULNERABLE) {
            caps.invulnerable = invulnerable;
        }
        if let Some(no_clip) = resolved(ability_bit::NO_CLIP) {
            caps.has_collision = !no_clip;
        }
        caps
    }

    /// Held survival/adventure mining runs here; creative's instant break does not.
    pub(crate) const fn uses_survival_mining(&self) -> bool {
        self.can_edit && !self.instant_break
    }
}

/// The effective value of one ability bit: the last received layer that defines
/// it wins. `None` means no layer defined it, so the mode default stands.
fn resolved_ability(update: &AbilitiesUpdate, bit: u32) -> Option<bool> {
    let AbilityLayersEvidence::Received(layers) = &update.layers else {
        return None;
    };
    let mut effective = None;
    for layer in layers.iter() {
        if layer.abilities & bit != 0 {
            effective = Some(layer.values & bit != 0);
        }
    }
    effective
}

#[cfg(test)]
mod tests;
