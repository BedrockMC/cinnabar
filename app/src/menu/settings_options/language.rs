//! The vanilla language list supplies its native names and ordering at runtime.

use std::{
    fs::File,
    io::Read,
    path::{Path, PathBuf},
    sync::Arc,
};

use super::SettingsOptions;
use crate::{menu::MenuRuntime, ui_runtime::UiRuntime};

const MAX_LANGUAGE_NAMES_BYTES: u64 = 64 * 1024;

impl SettingsOptions {
    /// An absent choice follows the startup locale.
    pub(crate) fn language(&self) -> Option<&str> {
        self.language.as_deref()
    }

    /// Accepts locale codes only; an unchanged choice needs no save or reload.
    pub(crate) fn set_language(&mut self, code: &str) -> bool {
        if !assets::is_language_code(code) || self.language() == Some(code) {
            return false;
        }
        self.language = Some(code.to_owned());
        true
    }

    /// Reads the installed pack's native language names without duplicating its list.
    pub(crate) fn language_choices(resource_root: &Path) -> Arc<[(String, String)]> {
        let path = resource_root
            .join(crate::install_layout::vanilla_pack_relative())
            .join("texts/language_names.json");
        let mut bytes = Vec::new();
        let Ok(file) = File::open(path) else {
            return Arc::from([]);
        };
        if file
            .take(MAX_LANGUAGE_NAMES_BYTES + 1)
            .read_to_end(&mut bytes)
            .is_err()
            || bytes.len() as u64 > MAX_LANGUAGE_NAMES_BYTES
        {
            return Arc::from([]);
        }
        parse_choices(&bytes).into()
    }
}

/// Drops malformed locale rows before their names reach the UI.
fn parse_choices(bytes: &[u8]) -> Vec<(String, String)> {
    serde_json::from_slice::<Vec<(String, String)>>(bytes)
        .unwrap_or_default()
        .into_iter()
        .filter(|(code, name)| assets::is_language_code(code) && !name.is_empty())
        .collect()
}

impl MenuRuntime {
    /// Uses the selected carrier path and gives a CLI locale priority over the saved choice.
    pub(crate) fn with_language_assets(mut self, path: PathBuf, requested: Option<&str>) -> Self {
        self.language_asset_path = path;
        let code =
            crate::asset_startup::active_language(requested.or(self.settings_options.language()));
        Arc::make_mut(&mut self.settings_options).set_language(&code);
        self.language_pending = true;
        self
    }

    /// Changes the language selected by the vanilla radio collection.
    pub(in crate::menu) fn set_language(&mut self, index: u16) {
        let Some((code, _)) = self.language_choices.get(usize::from(index)) else {
            return;
        };
        if Arc::make_mut(&mut self.settings_options).set_language(code) {
            self.settings_dirty = true;
            self.language_pending = true;
        }
    }

    /// Installs a selected carrier once; missing optional translations fall back to English.
    pub(crate) fn sync_language(&mut self, runtime: &mut UiRuntime) {
        if !self.language_pending {
            return;
        }
        self.language_pending = false;
        let catalog = crate::asset_startup::load_active_language(
            &self.language_asset_path,
            self.settings_options.language(),
        );
        runtime.set_active_language(catalog);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_language_order_and_names_come_from_the_pack() {
        assert_eq!(
            parse_choices(br#"[["de_DE","Deutsch"],["en_US","English"],["bad","Bad"]]"#),
            vec![
                ("de_DE".into(), "Deutsch".into()),
                ("en_US".into(), "English".into())
            ]
        );
    }

    #[test]
    fn language_default_and_persistence_round_trip() {
        let mut settings = SettingsOptions::default();
        assert_eq!(settings.language(), None);
        assert!(!settings.set_language("../bad"));
        assert!(settings.set_language("de_DE"));
        let bytes = serde_json::to_vec(&settings).unwrap();
        assert_eq!(
            SettingsOptions::decode(&bytes).unwrap().language(),
            Some("de_DE")
        );
    }

    #[test]
    fn selecting_a_language_updates_runtime_text_and_english_restores_the_base() {
        let directory =
            std::env::temp_dir().join(format!("cinnabar-language-{}", std::process::id()));
        std::fs::create_dir_all(directory.join("lang")).unwrap();
        let provenance = crate::asset_startup::canonical_source_manifest_sha256(
            crate::asset_startup::vanilla_source_manifest_json(),
        );
        let entries = [assets::LangEntry {
            key: "tile.stone.name".into(),
            value: "Stein".into(),
        }];
        let bytes = assets::encode_lang_catalog(provenance, [0; 32], &entries).unwrap();
        std::fs::write(directory.join("lang/de_DE.mcbelang"), bytes).unwrap();
        let mut menu = MenuRuntime::new(true, 2, "Steve".into())
            .with_language_assets(directory.join("world.mcbea"), Some("en_US"));
        menu.language_choices = Arc::from([
            ("en_US".to_owned(), "English".to_owned()),
            ("de_DE".to_owned(), "Deutsch".to_owned()),
        ]);
        let entries = [assets::LangEntry {
            key: "tile.stone.name".into(),
            value: "Stone".into(),
        }];
        let bytes = assets::encode_lang_catalog(provenance, [0; 32], &entries).unwrap();
        let mut runtime = UiRuntime::new(0);
        runtime.set_lang_catalog(Arc::new(
            assets::RuntimeLangCatalog::decode(&bytes).unwrap(),
        ));
        menu.set_language(1);
        menu.sync_language(&mut runtime);
        assert_eq!(runtime.localized_item_name("minecraft:stone"), "Stein");
        menu.set_language(0);
        menu.sync_language(&mut runtime);
        assert_eq!(runtime.localized_item_name("minecraft:stone"), "Stone");
        std::fs::remove_dir_all(directory).unwrap();
    }
}
