//! Renderer setup shared by direct and launcher sessions.

use std::num::NonZeroU32;

use bevy::{
    render::{
        RenderPlugin,
        settings::{RenderCreation, WgpuSettings},
    },
    window::{PresentMode, Window},
};

/// The primary window; `frame_latency` must suit the session's VSync choice from the start.
pub(super) fn primary_window(
    title: String,
    present_mode: PresentMode,
    frame_latency: NonZeroU32,
) -> Window {
    Window {
        title,
        present_mode,
        desired_maximum_frame_latency: Some(frame_latency),
        ..Default::default()
    }
}

pub(super) fn render_plugin() -> RenderPlugin {
    let mut settings = WgpuSettings::default();
    settings.limits.max_storage_buffers_per_shader_stage = settings
        .limits
        .max_storage_buffers_per_shader_stage
        .max(render::required_vertex_storage_buffers());
    if let Some(backends) =
        super::preferred_render_backends(std::env::var_os("WGPU_BACKEND").as_deref())
    {
        settings.backends = Some(backends);
    }
    RenderPlugin {
        render_creation: RenderCreation::Automatic(settings),
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn primary_window_queues_one_frame_with_vsync_and_two_without() {
        // Windows request FIFO until the surface is probed, whatever VSync choice they serve.
        for (vsync, queued) in [(true, 1), (false, 2)] {
            let window = primary_window(
                String::new(),
                PresentMode::Fifo,
                render::frame_latency_for_vsync(vsync),
            );
            assert_eq!(
                window.desired_maximum_frame_latency.map(NonZeroU32::get),
                Some(queued)
            );
            assert_eq!(window.present_mode, PresentMode::Fifo);
        }
    }
}
