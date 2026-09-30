use semantic_input::{ControlSettings, PerspectiveMode};

pub const CURRENT_SETTINGS_SCHEMA: u32 = 2;

#[derive(Clone, Debug, PartialEq)]
pub struct UserSettings {
    pub schema_version: u32,
    pub controls: ControlSettings,
    pub video: VideoSettings,
    pub gameplay: GameplaySettings,
}

impl Default for UserSettings {
    fn default() -> Self {
        Self {
            schema_version: CURRENT_SETTINGS_SCHEMA,
            controls: ControlSettings::default(),
            video: VideoSettings::default(),
            gameplay: GameplaySettings::default(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct VideoSettings {
    pub horizontal_fov_degrees: f32,
    pub fullscreen: bool,
    pub frame_cap: Option<u16>,
    pub vsync: bool,
    pub ui_scale: f32,
    pub render_distance_chunks: u8,
    pub brightness: f32,
    /// Scales speed-driven FOV changes, `0..=1`.
    pub fov_effects_scale: f32,
    /// Scales portal and nausea distortion, `0..=1`.
    pub distortion_scale: f32,
    pub view_bobbing: bool,
    pub cinematic_camera: bool,
}

impl Default for VideoSettings {
    fn default() -> Self {
        Self {
            horizontal_fov_degrees: 90.0,
            fullscreen: false,
            frame_cap: None,
            vsync: true,
            ui_scale: 1.0,
            render_distance_chunks: 16,
            brightness: 0.5,
            fov_effects_scale: 1.0,
            distortion_scale: 1.0,
            view_bobbing: true,
            cinematic_camera: false,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct GameplaySettings {
    pub default_perspective: PerspectiveMode,
    /// Sprint key toggles a persistent sprint instead of requiring hold.
    pub toggle_sprint: bool,
    /// Sneak key toggles a persistent sneak instead of requiring hold.
    pub toggle_sneak: bool,
}
