//! What each item does on an air use beyond the click-air transaction every item sends.

/// `BowItem`/`TridentItem::getMaxUseDuration`.
const LONG_USE_TICKS: u32 = 72_000;
const SPYGLASS_USE_TICKS: u32 = 1_200;
/// `CrossbowItem::getMaxUseDuration`: 25 ticks less 5 per Quick Charge level.
const CROSSBOW_CHARGE_TICKS: u32 = 25;
const QUICK_CHARGE_TICKS_PER_LEVEL: u32 = 5;
/// `EnderpearlItem::getCooldownDuration`.
const ENDER_PEARL_COOLDOWN: Cooldown = Cooldown {
    category: "ender_pearl",
    ticks: 20,
};
/// The vanilla pack's `wind_charge` `minecraft:cooldown` (0.5 s).
const WIND_CHARGE_COOLDOWN: Cooldown = Cooldown {
    category: "wind_charge",
    ticks: 10,
};
/// What a use needs before it starts outside creative.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Needs {
    Nothing,
    Arrow,
    /// Arrows anywhere, or a firework rocket in the offhand.
    ArrowOrOffhandRocket,
}

/// A use's shared cooldown, as `Player::startItemCooldown` records it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Cooldown {
    pub(crate) category: &'static str,
    pub(crate) ticks: u32,
}

/// What pressing use in the air does with the selected item.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum AirUse {
    /// Starts a use that ends on release, or silently once `max_ticks` run out.
    Hold { max_ticks: u32, needs: Needs },
    /// Consumes one item outside creative and swings, as a thrown projectile does.
    Throw { cooldown: Option<Cooldown> },
    /// Acts at once, as a loaded crossbow fires.
    Instant,
}

impl AirUse {
    const fn hold(max_ticks: u32, needs: Needs) -> Self {
        Self::Hold { max_ticks, needs }
    }

    pub(crate) const fn cooldown(self) -> Option<Cooldown> {
        match self {
            Self::Throw { cooldown } => cooldown,
            Self::Hold { .. } | Self::Instant => None,
        }
    }

    /// A crossbow acts only on a fresh press; everything else repeats while use is held.
    pub(crate) const fn repeats_while_held(self) -> bool {
        !matches!(
            self,
            Self::Instant
                | Self::Hold {
                    needs: Needs::ArrowOrOffhandRocket,
                    ..
                }
        )
    }
}

/// The air use of `identifier`; `None` sends only the click-air transaction.
pub(crate) fn classify(identifier: &str, charged: bool, quick_charge: u8) -> Option<AirUse> {
    let name = identifier.strip_prefix("minecraft:")?;
    Some(match name {
        "bow" => AirUse::hold(LONG_USE_TICKS, Needs::Arrow),
        "trident" => AirUse::hold(LONG_USE_TICKS, Needs::Nothing),
        "spyglass" => AirUse::hold(SPYGLASS_USE_TICKS, Needs::Nothing),
        "crossbow" if charged => AirUse::Instant,
        "crossbow" => AirUse::hold(
            CROSSBOW_CHARGE_TICKS
                .saturating_sub(u32::from(quick_charge) * QUICK_CHARGE_TICKS_PER_LEVEL),
            Needs::ArrowOrOffhandRocket,
        ),
        "snowball" | "egg" | "blue_egg" | "brown_egg" | "experience_bottle" | "splash_potion"
        | "lingering_potion" => AirUse::Throw { cooldown: None },
        "ender_pearl" => AirUse::Throw {
            cooldown: Some(ENDER_PEARL_COOLDOWN),
        },
        "wind_charge" => AirUse::Throw {
            cooldown: Some(WIND_CHARGE_COOLDOWN),
        },
        _ => return None,
    })
}
