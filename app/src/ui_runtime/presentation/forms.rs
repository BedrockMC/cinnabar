//! Launcher frame adapter around the extracted JSON-UI presentation.
#[cfg(test)]
pub(crate) use client_ui::ui_runtime::presentation::forms::snapshot;
pub use client_ui::ui_runtime::presentation::forms::{
    LoadingStage, ServerUiPack, built_in_faces, launcher_view, observe_station_block,
};
pub mod panorama;
pub(crate) use panorama::drive_menu_panorama;

#[cfg(test)]
pub mod pack_harness;

#[cfg(test)]
pub(crate) mod tests {
    pub(crate) use client_ui::test_support::mini_engine_presentation;
}

#[cfg(test)]
mod loading_sequence_tests;
#[cfg(test)]
mod menu_gpu_tests;
#[cfg(test)]
mod play_flow_snapshots {
    pub(crate) use client_ui::test_support::fixture_view;
}
