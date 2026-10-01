//! Subscriber fingerprints preserve unchanged compiled resources across stack edits.

use super::resource_packs::PackApplication;
use resource_pack::{PackAdmission, ValidatedPackStack};
use sha2::{Digest, Sha256};

pub(super) struct Changes {
    pub(super) blocks: bool,
    pub(super) atmosphere: bool,
    pub(super) particles: bool,
    pub(super) icons: bool,
    pub(super) glyphs: bool,
    pub(super) entities: bool,
    pub(super) ui: bool,
    pub(super) sounds: bool,
    pub(super) language: bool,
}

impl Changes {
    /// Compares layer contents, including order, rather than pack names or timestamps.
    pub(super) fn between(stack: &ValidatedPackStack, previous: Option<&PackApplication>) -> Self {
        let prior = previous.and_then(|old| match &old.admission {
            PackAdmission::Validated(stack) => Some(stack.as_ref()),
            PackAdmission::None => None,
        });
        let changed = |prefixes: &[&str]| {
            prior.is_none_or(|old| fingerprint(old, prefixes) != fingerprint(stack, prefixes))
        };
        Self {
            blocks: changed(&["textures/", "models/", "blocks/", "biomes/"]),
            atmosphere: changed(&["textures/environment/", "biomes/", "fogs/"]),
            particles: changed(&["particles/", "textures/"]),
            icons: changed(&["textures/", "models/", "blocks/", "items/"]),
            glyphs: changed(&["font/"]),
            entities: changed(&[
                "entity/",
                "attachables/",
                "animations/",
                "animation_controllers/",
                "render_controllers/",
                "models/",
                "materials/",
                "textures/",
            ]),
            ui: changed(&["ui/", "textures/"]),
            sounds: changed(&["sounds/", "sounds.json"]),
            language: changed(&["texts/"]),
        }
    }
}

/// Empty layers do not affect a subscriber, while file-bearing layer order does.
fn fingerprint(stack: &ValidatedPackStack, prefixes: &[&str]) -> [u8; 32] {
    let mut hash = Sha256::new();
    for pack in stack.packs() {
        let mut layer = Sha256::new();
        let mut populated = false;
        let mut paths = std::collections::BTreeSet::new();
        for prefix in prefixes {
            paths.extend(pack.files_under(prefix));
        }
        for path in paths {
            if let Ok(Some(bytes)) = pack.read_file(path) {
                layer.update((path.len() as u64).to_le_bytes());
                layer.update(path.as_bytes());
                layer.update((bytes.len() as u64).to_le_bytes());
                layer.update(bytes);
                populated = true;
            }
        }
        if populated {
            hash.update(layer.finalize());
        }
    }
    hash.finalize().into()
}

#[cfg(test)]
#[path = "pack_reload_diff_tests.rs"]
mod tests;
