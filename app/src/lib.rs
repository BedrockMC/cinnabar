pub mod args;
pub mod asset_startup;
mod block_cracks;
mod block_entities;
mod block_use;
pub mod camera;
mod environment;
mod first_run;
mod game_mode_capabilities;
mod hotbar;
mod install_layout;
mod interaction_authority;
pub mod lifecycle;
pub mod local_player;
mod local_player_camera_receipt;
mod melee;
mod menu;
pub mod metrics;
mod mining;
pub mod movement;
mod named_audio;
mod native_dialog;
mod particles;
mod player_skin;
mod present_mode;
pub mod semantic_controls;
pub mod server_camera;
pub mod session_audio;
mod session_cleanup;
pub mod settings_runtime;
mod survival_mining;
pub mod ui_runtime;

mod acceptance;
mod app;
mod presentation;
mod runtime;

pub use app::run;

#[cfg(test)]
mod tests;
