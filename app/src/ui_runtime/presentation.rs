use super::UiRuntime;
use crate::{
    camera::CameraSettingsAuthority,
    runtime::{
        shutdown::record_fatal_error,
        visibility::CaveVisibilityCache,
        world::{ClientWorld, WorldStreamFramePoll},
    },
};
use bevy::{
    camera::Camera,
    prelude::{Camera3d, GlobalTransform, Query, Res, ResMut, Time, With},
    time::Real,
    window::{PrimaryWindow, Window},
};
use render::{
    ChunkRenderQueue, ChunkUploadAcknowledgements, UiRenderScene, UiRenderStats,
    VisibilityDiagnostics, VisibilityDiagnosticsInput,
};
use std::sync::Arc;
use ui::{DpiScale, SafeArea};

#[cfg(test)]
pub(crate) use client_ui::ui_runtime::presentation::refresh_hud_frame;
pub use client_ui::ui_runtime::presentation::{
    BUILT_IN_TITLE, BedHit, ChatHit, DebugLines, HudFrame, IconRef, LoadingStage,
    MAX_PACK_TEXTURE_BYTES, MAX_SESSION_ICON_SIDE, PreparedUiPublication, ServerUiPack,
    SessionGlyphSheets, SessionIcon, SessionIcons, UiPresentationError, UiPresentationRuntime,
    inventory_pointer, menu_artwork, nametag_atlas, nametags,
};
pub mod forms;
pub mod gui_scale_settings;
pub mod publish;
pub(crate) use forms::drive_menu_panorama;
pub(crate) use gui_scale_settings::apply_gui_scale_setting;
pub(crate) use publish::{
    observe_mount_jump_input, platform_safe_area_insets, prepare_ui_runtime, publish_ui_runtime,
};

#[cfg(test)]
pub(crate) mod tests;
