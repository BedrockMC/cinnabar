//! Render-mode setting: saved choice, session overrides, and the per-camera
//! Enhanced opt-in. Vanilla stays the default and the evidence-run mode.

use std::{
    ffi::OsStr,
    fs,
    io::Read,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result};
use bevy::prelude::*;
use render::{EnhancedRenderPlugin, EnhancedRendering};
use serde::{Deserialize, Serialize};
use ui::RenderMode;

use crate::{camera::FlyCamera, menu::MenuRuntime, settings_runtime::RuntimeSettings};

pub(crate) const RENDER_MODE_ENV: &str = "CINNABAR_RENDER_MODE";
const MAX_GRAPHICS_FILE_BYTES: u64 = 4096;

#[derive(Serialize, Deserialize)]
struct GraphicsFile {
    render_mode: String,
}

/// Saved mode, or `None` when the file is absent, oversized, or unreadable.
pub(crate) fn load_render_mode(path: &Path) -> Option<RenderMode> {
    let mut bytes = Vec::new();
    fs::File::open(path)
        .ok()?
        .take(MAX_GRAPHICS_FILE_BYTES + 1)
        .read_to_end(&mut bytes)
        .ok()?;
    if bytes.len() as u64 > MAX_GRAPHICS_FILE_BYTES {
        return None;
    }
    let file = serde_json::from_slice::<GraphicsFile>(&bytes).ok()?;
    RenderMode::parse(&file.render_mode)
}

/// Atomically replace the small graphics extension settings file.
pub(crate) fn save_render_mode(path: &Path, mode: RenderMode) -> Result<()> {
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
    }
    let bytes = serde_json::to_vec_pretty(&GraphicsFile {
        render_mode: mode.as_str().to_owned(),
    })?;
    let temp = path.with_extension("json.tmp");
    fs::write(&temp, bytes).with_context(|| format!("write {}", temp.display()))?;
    fs::rename(&temp, path).with_context(|| format!("replace {}", path.display()))
}

/// CLI beats environment beats the saved file. Attributable evidence runs
/// ignore the environment and saved file so captures stay vanilla.
pub(crate) fn startup_render_mode(
    cli: Option<RenderMode>,
    env: Option<&OsStr>,
    saved: Option<RenderMode>,
    attributable: bool,
) -> RenderMode {
    if let Some(mode) = cli {
        return mode;
    }
    if attributable {
        return RenderMode::Vanilla;
    }
    env.and_then(OsStr::to_str)
        .and_then(RenderMode::parse)
        .or(saved)
        .unwrap_or_default()
}

pub(crate) struct RenderModePlugin {
    cli: Option<RenderMode>,
    attributable: bool,
}

impl RenderModePlugin {
    /// Configure session overrides and deterministic evidence runs.
    pub(crate) const fn new(cli: Option<RenderMode>, attributable: bool) -> Self {
        Self { cli, attributable }
    }
}

#[derive(Resource)]
struct RenderModeConfig {
    cli: Option<RenderMode>,
    attributable: bool,
    path: Option<PathBuf>,
}

impl Plugin for RenderModePlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(EnhancedRenderPlugin)
            .insert_resource(RenderModeConfig {
                cli: self.cli,
                attributable: self.attributable,
                path: None,
            })
            .add_systems(Startup, seed_render_mode)
            .add_systems(
                Update,
                (apply_menu_render_mode, apply_render_mode_to_cameras).chain(),
            );
    }
}

/// Resolve the saved choice and session overrides at startup.
fn seed_render_mode(
    mut config: ResMut<RenderModeConfig>,
    menu: Option<Res<MenuRuntime>>,
    mut settings: ResMut<RuntimeSettings>,
) {
    config.path = menu.map(|menu| menu.graphics_file());
    let saved = config.path.as_deref().and_then(load_render_mode);
    let mode = startup_render_mode(
        config.cli,
        std::env::var_os(RENDER_MODE_ENV).as_deref(),
        saved,
        config.attributable,
    );
    set_render_mode(&mut settings, mode);
}

/// Update the shared settings authority only when the choice changes.
fn set_render_mode(settings: &mut RuntimeSettings, mode: RenderMode) {
    let (_, current) = settings.user_settings_update();
    if current.video.render_mode != mode {
        let mut next = current.clone();
        next.video.render_mode = mode;
        settings.replace_user_settings(next);
    }
}

/// Persist menu changes and keep the displayed toggle synchronized.
fn apply_menu_render_mode(
    config: Res<RenderModeConfig>,
    menu: Option<ResMut<MenuRuntime>>,
    mut settings: ResMut<RuntimeSettings>,
) {
    let Some(mut menu) = menu else {
        return;
    };
    if let Some(mode) = menu.take_render_mode_request() {
        set_render_mode(&mut settings, mode);
        if let Some(path) = &config.path
            && let Err(error) = save_render_mode(path, mode)
        {
            warn!(?error, "render mode could not be saved");
        }
    }
    menu.sync_render_mode(settings.user_settings_update().1.video.render_mode);
}

/// Keep the opt-in marker on gameplay cameras only.
fn apply_render_mode_to_cameras(
    mut commands: Commands,
    settings: Res<RuntimeSettings>,
    cameras: Query<(Entity, Has<EnhancedRendering>), With<FlyCamera>>,
) {
    let enhanced = settings.user_settings_update().1.video.render_mode == RenderMode::Enhanced;
    for (entity, has_enhanced) in &cameras {
        if enhanced != has_enhanced {
            if enhanced {
                commands.entity(entity).insert(EnhancedRendering::default());
            } else {
                commands.entity(entity).remove::<EnhancedRendering>();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cli_overrides_env_and_saved_mode_and_evidence_runs_stay_vanilla() {
        let enhanced = Some(OsStr::new("enhanced"));
        assert_eq!(
            startup_render_mode(None, None, None, false),
            RenderMode::Vanilla
        );
        assert_eq!(
            startup_render_mode(None, None, Some(RenderMode::Enhanced), false),
            RenderMode::Enhanced
        );
        assert_eq!(
            startup_render_mode(
                None,
                Some(OsStr::new("vanilla")),
                Some(RenderMode::Enhanced),
                false
            ),
            RenderMode::Vanilla
        );
        assert_eq!(
            startup_render_mode(Some(RenderMode::Vanilla), enhanced, None, false),
            RenderMode::Vanilla
        );
        assert_eq!(
            startup_render_mode(None, enhanced, Some(RenderMode::Enhanced), true),
            RenderMode::Vanilla
        );
        assert_eq!(
            startup_render_mode(Some(RenderMode::Enhanced), None, None, true),
            RenderMode::Enhanced
        );
        assert_eq!(
            startup_render_mode(None, Some(OsStr::new("bogus")), None, false),
            RenderMode::Vanilla
        );
    }

    #[test]
    fn saved_render_mode_round_trips_and_rejects_malformed_files() {
        let directory = std::env::temp_dir().join(format!(
            "cinnabar-render-mode-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let path = directory.join("nested/graphics.json");
        assert_eq!(load_render_mode(&path), None);
        save_render_mode(&path, RenderMode::Enhanced).expect("save");
        assert_eq!(load_render_mode(&path), Some(RenderMode::Enhanced));
        save_render_mode(&path, RenderMode::Vanilla).expect("save");
        assert_eq!(load_render_mode(&path), Some(RenderMode::Vanilla));
        fs::write(&path, br#"{"render_mode":"ultra"}"#).expect("write");
        assert_eq!(load_render_mode(&path), None);
        fs::write(&path, vec![b' '; MAX_GRAPHICS_FILE_BYTES as usize + 1]).expect("write");
        assert_eq!(load_render_mode(&path), None);
        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn toggling_the_setting_adds_and_removes_the_camera_opt_in() {
        let mut app = App::new();
        app.init_resource::<RuntimeSettings>()
            .insert_resource(RenderModeConfig {
                cli: None,
                attributable: false,
                path: None,
            })
            .add_systems(Update, apply_render_mode_to_cameras);
        let camera = app
            .world_mut()
            .spawn((Camera3d::default(), FlyCamera::default()))
            .id();
        app.update();
        assert!(app.world().get::<EnhancedRendering>(camera).is_none());

        set_render_mode(
            &mut app.world_mut().resource_mut::<RuntimeSettings>(),
            RenderMode::Enhanced,
        );
        app.update();
        assert!(app.world().get::<EnhancedRendering>(camera).is_some());
        set_render_mode(
            &mut app.world_mut().resource_mut::<RuntimeSettings>(),
            RenderMode::Vanilla,
        );
        app.update();
        assert!(app.world().get::<EnhancedRendering>(camera).is_none());
    }
}
