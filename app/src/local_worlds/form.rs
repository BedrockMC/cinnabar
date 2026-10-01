use protocol::world_control::{Backend, Difficulty, GameMode, Generator, NewWorld, World};

/// Vanilla's world name field limit (`CreateNewWorld.general`, 30 characters); the core allows 64.
pub(crate) const MAX_WORLD_NAME_CHARS: usize = 30;
/// Vanilla's seed field limit.
pub(crate) const MAX_SEED_CHARS: usize = 32;
const DEFAULT_WORLD_NAME: &str = "My World";

/// Editable settings of the create-world screen.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct CreateForm {
    pub(crate) name: String,
    pub(crate) game_mode: GameMode,
    pub(crate) generator: Generator,
    pub(crate) difficulty: Difficulty,
    /// Blank means random; digits are used as-is and other text is hashed.
    pub(crate) seed_text: String,
}

impl Default for CreateForm {
    fn default() -> Self {
        Self {
            name: DEFAULT_WORLD_NAME.to_owned(),
            game_mode: GameMode::Survival,
            generator: Generator::Normal,
            difficulty: Difficulty::Normal,
            seed_text: String::new(),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum FormError {
    EmptyName,
    NameTooLong,
    ControlCharacters,
}

impl FormError {
    pub(crate) fn message(self) -> &'static str {
        match self {
            Self::EmptyName => "Enter a world name",
            Self::NameTooLong => "World name is too long",
            Self::ControlCharacters => "World name has invalid characters",
        }
    }
}

/// Trims and validates a world name against the core's limits.
pub(crate) fn validate_name(name: &str) -> Result<String, FormError> {
    let name = name.trim();
    if name.is_empty() {
        return Err(FormError::EmptyName);
    }
    if name.chars().count() > MAX_WORLD_NAME_CHARS {
        return Err(FormError::NameTooLong);
    }
    if name.chars().any(char::is_control) {
        return Err(FormError::ControlCharacters);
    }
    Ok(name.to_owned())
}

/// Numeric text is the seed; other text is hashed (FNV-1a); blank is random.
pub(crate) fn seed_from_text(text: &str) -> Option<i64> {
    let text = text.trim();
    if text.is_empty() {
        return None;
    }
    if let Ok(seed) = text.parse::<i64>() {
        return Some(seed);
    }
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    for byte in text.bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    Some(i64::from_ne_bytes(hash.to_ne_bytes()))
}

/// Editable settings of a saved world.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct EditForm {
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) game_mode: GameMode,
    pub(crate) difficulty: Difficulty,
}

impl EditForm {
    pub(crate) fn of(world: &World) -> Self {
        Self {
            id: world.id.clone(),
            name: world.name.clone(),
            game_mode: world.game_mode,
            difficulty: world.difficulty,
        }
    }
}

impl CreateForm {
    pub(crate) fn build(&self) -> Result<NewWorld, FormError> {
        Ok(NewWorld {
            name: validate_name(&self.name)?,
            game_mode: self.game_mode,
            generator: self.generator,
            difficulty: self.difficulty,
            // Flat worlds run on the built-in server, which can pause; default worlds need BDS.
            backend: (self.generator == Generator::Flat).then_some(Backend::Dragonfly),
            seed: seed_from_text(&self.seed_text),
        })
    }
}

pub(crate) fn game_mode_label(mode: GameMode) -> &'static str {
    match mode {
        GameMode::Survival => "Survival",
        GameMode::Creative => "Creative",
        GameMode::Adventure => "Adventure",
    }
}

/// Vanilla's per-mode description under the game mode control.
pub(crate) fn game_mode_description(mode: GameMode) -> &'static str {
    match mode {
        GameMode::Survival => {
            "Explore a mysterious world where you build, collect, craft, and fight monsters."
        }
        GameMode::Creative => {
            "Create, build, and explore without limits. You can fly, have endless materials, and \
             can't be hurt by monsters."
        }
        GameMode::Adventure => {
            "You get to set your own rules through in-game commands on how you and other can \
             interact with the game."
        }
    }
}

/// Vanilla's per-difficulty description under the difficulty control.
pub(crate) fn difficulty_description(difficulty: Difficulty) -> &'static str {
    match difficulty {
        Difficulty::Peaceful => {
            "No hostile mobs and only some neutral mobs spawn. Hunger bar doesn't deplete and \
             health replenishes over time."
        }
        Difficulty::Easy => {
            "Hostile mobs spawn but deal less damage. Hunger bar depletes and drains health down \
             to 5 hearts."
        }
        Difficulty::Normal => {
            "Hostile mobs spawn and deal standard damage. Hunger bar depletes and drains health \
             down to half a heart."
        }
        Difficulty::Hard => {
            "Hostile mobs spawn and deal more damage. Hunger bar depletes and drains all health."
        }
    }
}

pub(crate) fn difficulty_label(difficulty: Difficulty) -> &'static str {
    match difficulty {
        Difficulty::Peaceful => "Peaceful",
        Difficulty::Easy => "Easy",
        Difficulty::Normal => "Normal",
        Difficulty::Hard => "Hard",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_build_a_random_seed_survival_world() {
        let world = CreateForm::default().build().expect("valid defaults");
        assert_eq!(world.name, "My World");
        assert_eq!(world.game_mode, GameMode::Survival);
        assert_eq!(world.seed, None);
        assert_eq!(world.backend, None, "the core puts default worlds on BDS");
        let flat = CreateForm {
            generator: Generator::Flat,
            ..CreateForm::default()
        };
        assert_eq!(
            flat.build().map(|w| w.backend),
            Ok(Some(Backend::Dragonfly))
        );
    }

    #[test]
    fn name_is_trimmed_and_validated() {
        assert_eq!(validate_name("  Home  "), Ok("Home".to_owned()));
        assert_eq!(validate_name("   "), Err(FormError::EmptyName));
        assert_eq!(validate_name(&"x".repeat(31)), Err(FormError::NameTooLong));
        assert_eq!(validate_name(&"x".repeat(30)).map(|n| n.len()), Ok(30));
        assert_eq!(validate_name("a\nb"), Err(FormError::ControlCharacters));
    }

    #[test]
    fn seed_text_numeric_zero_negative_blank_and_hashed() {
        assert_eq!(seed_from_text(""), None);
        assert_eq!(seed_from_text("  "), None);
        assert_eq!(seed_from_text("0"), Some(0));
        assert_eq!(seed_from_text(" -42 "), Some(-42));
        let hashed = seed_from_text("glacier");
        assert!(hashed.is_some());
        assert_eq!(hashed, seed_from_text("glacier"));
        assert_ne!(hashed, seed_from_text("glacier2"));
    }
}
