use protocol::world_control::{Difficulty, GameMode, Generator, NewWorld};

const MAX_WORLD_NAME_CHARS: usize = 64;
const DEFAULT_WORLD_NAME: &str = "New World";

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

impl CreateForm {
    pub(crate) fn build(&self) -> Result<NewWorld, FormError> {
        Ok(NewWorld {
            name: validate_name(&self.name)?,
            game_mode: self.game_mode,
            generator: self.generator,
            difficulty: self.difficulty,
            backend: None,
            seed: seed_from_text(&self.seed_text),
        })
    }

    pub(crate) fn cycle_game_mode(&mut self) {
        self.game_mode = match self.game_mode {
            GameMode::Survival => GameMode::Creative,
            GameMode::Creative => GameMode::Adventure,
            GameMode::Adventure => GameMode::Survival,
        };
    }

    pub(crate) fn cycle_generator(&mut self) {
        self.generator = match self.generator {
            Generator::Normal => Generator::Flat,
            Generator::Flat => Generator::Normal,
        };
    }

    pub(crate) fn cycle_difficulty(&mut self) {
        self.difficulty = match self.difficulty {
            Difficulty::Peaceful => Difficulty::Easy,
            Difficulty::Easy => Difficulty::Normal,
            Difficulty::Normal => Difficulty::Hard,
            Difficulty::Hard => Difficulty::Peaceful,
        };
    }
}

pub(crate) fn game_mode_label(mode: GameMode) -> &'static str {
    match mode {
        GameMode::Survival => "Survival",
        GameMode::Creative => "Creative",
        GameMode::Adventure => "Adventure",
    }
}

pub(crate) fn generator_label(generator: Generator) -> &'static str {
    match generator {
        Generator::Normal => "Default",
        Generator::Flat => "Superflat",
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
        assert_eq!(world.name, "New World");
        assert_eq!(world.game_mode, GameMode::Survival);
        assert_eq!(world.seed, None);
    }

    #[test]
    fn name_is_trimmed_and_validated() {
        assert_eq!(validate_name("  Home  "), Ok("Home".to_owned()));
        assert_eq!(validate_name("   "), Err(FormError::EmptyName));
        assert_eq!(validate_name(&"x".repeat(65)), Err(FormError::NameTooLong));
        assert_eq!(validate_name(&"x".repeat(64)).map(|n| n.len()), Ok(64));
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

    #[test]
    fn cycles_visit_every_value_and_wrap() {
        let mut form = CreateForm::default();
        let modes: Vec<_> = (0..3)
            .map(|_| {
                form.cycle_game_mode();
                form.game_mode
            })
            .collect();
        assert_eq!(
            modes,
            [GameMode::Creative, GameMode::Adventure, GameMode::Survival]
        );
        form.cycle_generator();
        assert_eq!(form.generator, Generator::Flat);
        form.cycle_generator();
        assert_eq!(form.generator, Generator::Normal);
        let mut seen = vec![form.difficulty];
        for _ in 0..3 {
            form.cycle_difficulty();
            seen.push(form.difficulty);
        }
        seen.dedup();
        assert_eq!(seen.len(), 4);
        form.cycle_difficulty();
        assert_eq!(form.difficulty, Difficulty::Normal);
    }
}
