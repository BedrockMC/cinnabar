use bevy::prelude::Shader;
use bytemuck::{Pod, Zeroable};

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub(super) struct UiViewportUniform {
    pub(super) viewport_size: [f32; 2],
    /// Seconds since the UI renderer started; animates the item glint.
    pub(super) time_seconds: f32,
    pub(super) glint_strength: f32,
}

pub(crate) fn source(raw: &str) -> String {
    // Keep the GPU style bit owned by the renderer, with no UI-crate dependency or WGSL copy.
    raw.replace(
        "UI_STYLE_ALPHA_TEST",
        &format!("{}u", crate::ui::UI_STYLE_ALPHA_TEST),
    )
}

pub(super) fn from_wgsl(raw: &str, path: impl Into<String>) -> Shader {
    Shader::from_wgsl(source(raw), path)
}
