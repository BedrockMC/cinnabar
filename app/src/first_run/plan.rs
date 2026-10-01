//! Ordered asset-preparation steps, mirroring the `make assets` recipe against the pinned manifests.

use std::path::Path;

use anyhow::{Context, Result};
use serde::Deserialize;

/// Carriers the production runtime refuses to start without.
pub(super) const REQUIRED_CARRIERS: &[&str] = &[
    "vanilla-v2193.mcbea",
    "vanilla-v1.mcbeatm",
    "vanilla-v1.mcbeent",
    "vanilla-v1.mcbehud",
    "vanilla-v1.mcbeico",
    "vanilla-v1.mcbelang",
    "vanilla-v1.mcbeui",
    "ui-monocraft-v1.mcbefont",
    "ui-font-notices.txt",
];

pub(super) const COMPILED: &str = ".local/assets/compiled";
pub(super) const VANILLA_MANIFEST: &str = "assets/vanilla-source.json";
const HUD_MANIFEST: &str = "assets/hud-source-v2193.json";
const FONT_MANIFEST: &str = "assets/ui-font-source.json";
const REGISTRY_DIR: &str = "crates/assets/data";

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum Action {
    /// Bundled fetch script under the workspace `scripts/` directory.
    Script(&'static str),
    /// `assetc` subcommand plus arguments, relative to the workspace.
    Assetc(Vec<String>),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct Step {
    pub label: &'static str,
    pub action: Action,
    /// A failed optional step is logged and skipped; its carrier degrades gracefully at runtime.
    pub required: bool,
}

#[derive(Deserialize)]
struct VanillaSource {
    cache_dir: String,
}

#[derive(Deserialize)]
struct FontSource {
    commit: String,
    font_file: String,
    fallback_commit: String,
    fallback_font_file: String,
}

/// Where the fetch script extracts the pinned pack, relative to the workspace.
pub(super) fn cache_dir(root: &Path) -> Result<String> {
    let vanilla: VanillaSource = read_json(&root.join(VANILLA_MANIFEST))?;
    Ok(vanilla.cache_dir)
}

/// The bundled UI font's file name, from the kit's copy of the font manifest.
pub(super) fn ui_font_file(kit: &Path) -> Result<String> {
    let font: FontSource = read_json(&kit.join(FONT_MANIFEST))?;
    Ok(font.font_file)
}

pub(super) fn carriers_present(dir: &Path) -> bool {
    REQUIRED_CARRIERS
        .iter()
        .all(|name| dir.join(name).is_file())
}

/// Builds the plan from the manifests already copied into `workspace`.
pub(super) fn steps(workspace: &Path) -> Result<Vec<Step>> {
    let vanilla: VanillaSource = read_json(&workspace.join(VANILLA_MANIFEST))?;
    let font: FontSource = read_json(&workspace.join(FONT_MANIFEST))?;
    let pack = format!("{}/resource_pack", vanilla.cache_dir);
    let out = |name: &str| format!("{COMPILED}/{name}");
    let assetc = |command: &str, extra: Vec<(&str, String)>| {
        let mut args = vec![command.to_owned()];
        for (flag, value) in extra {
            args.push(format!("--{flag}"));
            args.push(value);
        }
        Action::Assetc(args)
    };
    let pack_step = |command: &str, manifest: &str, blob: &str, report: &str| {
        assetc(
            command,
            vec![
                ("pack", pack.clone()),
                ("source-manifest", manifest.to_owned()),
                ("out", out(blob)),
                ("report", out(report)),
            ],
        )
    };
    let font_dir = format!(".local/assets/ui-font/{}", font.commit);
    let fallback_dir = format!(".local/assets/ui-font/{}", font.fallback_commit);
    let step = |label, action, required| Step {
        label,
        action,
        required,
    };
    Ok(vec![
        step(
            "Unpacking the Minecraft sample resource pack",
            Action::Script("fetch-vanilla-assets"),
            true,
        ),
        step(
            "Downloading the open UI font",
            Action::Script("fetch-ui-font"),
            true,
        ),
        step(
            "Compiling world assets",
            assetc(
                "compile",
                vec![
                    ("pack", pack.clone()),
                    ("source-manifest", VANILLA_MANIFEST.to_owned()),
                    (
                        "registry",
                        format!("{REGISTRY_DIR}/block-registry-v2193.bin"),
                    ),
                    (
                        "light-registry",
                        format!("{REGISTRY_DIR}/block-light-registry-v2193.bin"),
                    ),
                    (
                        "biome-registry",
                        format!("{REGISTRY_DIR}/biome-registry-v2193.bin"),
                    ),
                    ("out", out("vanilla-v2193.mcbea")),
                ],
            ),
            true,
        ),
        step(
            "Compiling sky assets",
            assetc(
                "atmosphere",
                vec![
                    ("pack", pack.clone()),
                    ("source-manifest", VANILLA_MANIFEST.to_owned()),
                    ("out", out("vanilla-v1.mcbeatm")),
                    ("report", out("atmosphere-assets.json")),
                ],
            ),
            true,
        ),
        step(
            "Compiling entity assets",
            pack_step(
                "entity-assets",
                VANILLA_MANIFEST,
                "vanilla-v1.mcbeent",
                "entity-assets.json",
            ),
            true,
        ),
        step(
            "Compiling the UI font",
            assetc(
                "outline-font-assets",
                vec![
                    ("font", format!("{font_dir}/{}", font.font_file)),
                    (
                        "fallback-font",
                        format!("{fallback_dir}/{}", font.fallback_font_file),
                    ),
                    ("source-manifest", FONT_MANIFEST.to_owned()),
                    ("out", out("ui-monocraft-v1.mcbefont")),
                    ("report", out("ui-monocraft-font-assets.json")),
                ],
            ),
            true,
        ),
        step(
            "Compiling HUD sprites",
            pack_step(
                "hud-assets",
                HUD_MANIFEST,
                "vanilla-v1.mcbehud",
                "hud-assets.json",
            ),
            true,
        ),
        step(
            "Compiling language files",
            pack_step(
                "lang-assets",
                VANILLA_MANIFEST,
                "vanilla-v1.mcbelang",
                "lang-assets.json",
            ),
            true,
        ),
        step(
            "Compiling other languages",
            assetc(
                "language-assets",
                vec![
                    ("pack", pack.clone()),
                    ("source-manifest", VANILLA_MANIFEST.to_owned()),
                    ("out-dir", out("lang")),
                ],
            ),
            false,
        ),
        step(
            "Compiling item icons",
            assetc(
                "icon-assets",
                vec![
                    ("pack", pack.clone()),
                    ("source-manifest", VANILLA_MANIFEST.to_owned()),
                    ("out", out("vanilla-v1.mcbeico")),
                    ("report", out("icon-assets.json")),
                    ("block-assets", out("vanilla-v2193.mcbea")),
                ],
            ),
            true,
        ),
        step(
            "Compiling audio catalog",
            pack_step(
                "audio-assets",
                VANILLA_MANIFEST,
                "vanilla-v1.mcbeaud",
                "audio-assets.json",
            ),
            false,
        ),
        step(
            "Compiling actor assets",
            pack_step(
                "actor-assets",
                VANILLA_MANIFEST,
                "vanilla-v1.mcbeact",
                "actor-assets.json",
            ),
            false,
        ),
        step(
            "Compiling sound bank",
            assetc(
                "audio-bank",
                vec![
                    ("pack", pack.clone()),
                    ("out", out("vanilla-v1.mcbesnd")),
                    ("report", out("audio-bank.json")),
                ],
            ),
            false,
        ),
        step(
            "Compiling equipment assets",
            assetc(
                "equipment-assets",
                vec![
                    ("pack", pack.clone()),
                    ("source-manifest", VANILLA_MANIFEST.to_owned()),
                    ("out", out("vanilla-v1.mcbeeqp")),
                    ("report", out("equipment-assets.json")),
                    (
                        "behavior-pack",
                        format!("{}/behavior_pack", vanilla.cache_dir),
                    ),
                ],
            ),
            false,
        ),
        step(
            "Compiling UI textures",
            pack_step(
                "ui-assets",
                VANILLA_MANIFEST,
                "vanilla-v1.mcbeui",
                "ui-assets.json",
            ),
            true,
        ),
        step(
            "Compiling particles",
            pack_step(
                "particle-assets",
                VANILLA_MANIFEST,
                "vanilla-v1.mcbept",
                "particle-assets.json",
            ),
            false,
        ),
        step(
            "Compiling block entities",
            pack_step(
                "block-entity-assets",
                VANILLA_MANIFEST,
                "vanilla-v1.mcbeben",
                "block-entity-assets.json",
            ),
            false,
        ),
        step(
            "Compiling weather textures",
            assetc(
                "weather-assets",
                vec![("pack", pack.clone()), ("out", out("vanilla-v1.mcbewth"))],
            ),
            false,
        ),
        step(
            "Compiling HUD extras",
            assetc(
                "hud-extras-assets",
                vec![("pack", pack.clone()), ("out", out("vanilla-v1.mcbehxt"))],
            ),
            false,
        ),
    ])
}

fn read_json<T: for<'de> Deserialize<'de>>(path: &Path) -> Result<T> {
    let bytes = std::fs::read(path).with_context(|| format!("read {}", path.display()))?;
    serde_json::from_slice(&bytes).with_context(|| format!("parse {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::first_run::test_support::Dir;

    fn workspace() -> Dir {
        let dir = Dir::new("plan");
        std::fs::create_dir_all(dir.path().join("assets")).unwrap();
        std::fs::write(
            dir.path().join(VANILLA_MANIFEST),
            r#"{"cache_dir":".local/assets/bedrock-samples/v1/full"}"#,
        )
        .unwrap();
        std::fs::write(
            dir.path().join(FONT_MANIFEST),
            r#"{"commit":"aa","font_file":"F.ttf","fallback_commit":"bb","fallback_font_file":"N.otf"}"#,
        )
        .unwrap();
        dir
    }

    #[test]
    fn plan_fetches_before_compiling_and_reads_pack_from_manifest() {
        let dir = workspace();
        let plan = steps(dir.path()).unwrap();
        assert!(matches!(
            plan[0].action,
            Action::Script("fetch-vanilla-assets")
        ));
        assert!(matches!(plan[1].action, Action::Script("fetch-ui-font")));
        let Action::Assetc(args) = &plan[2].action else {
            panic!("world compile must follow the fetches");
        };
        assert!(args.contains(&".local/assets/bedrock-samples/v1/full/resource_pack".to_owned()));
    }

    #[test]
    fn every_required_carrier_is_produced_by_a_required_step() {
        let dir = workspace();
        let plan = steps(dir.path()).unwrap();
        for carrier in REQUIRED_CARRIERS
            .iter()
            .filter(|name| name.contains("mcbe"))
        {
            let produced = plan.iter().any(|step| {
                step.required
                    && matches!(&step.action, Action::Assetc(args)
                        if args.iter().any(|arg| arg.ends_with(carrier)))
            });
            assert!(produced, "{carrier} has no required producer");
        }
    }

    #[test]
    fn icon_step_follows_the_world_step_it_reads() {
        let dir = workspace();
        let plan = steps(dir.path()).unwrap();
        let position = |command: &str| {
            plan.iter()
                .position(|step| matches!(&step.action, Action::Assetc(args) if args[0] == command))
        };
        assert!(position("compile") < position("icon-assets"));
    }

    #[test]
    fn carriers_present_requires_every_required_file() {
        let dir = Dir::new("carriers");
        assert!(!carriers_present(dir.path()));
        for name in REQUIRED_CARRIERS {
            std::fs::write(dir.path().join(name), b"x").unwrap();
        }
        assert!(carriers_present(dir.path()));
        std::fs::remove_file(dir.path().join(REQUIRED_CARRIERS[0])).unwrap();
        assert!(!carriers_present(dir.path()));
    }
}
