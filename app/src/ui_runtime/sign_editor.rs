//! App resources needed to open and drive the extracted sign editor.
pub use client_ui::ui_runtime::sign_editor::{MAX_LINE_DESIGN_PIXELS, SignEdit};
mod input;
pub(crate) use input::drive_sign_editor;
