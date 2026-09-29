//! Function-key client tools: screenshot capture (F2) and the debug overlay (F3).

mod screenshot;

use std::path::PathBuf;

use bevy::prelude::{App, Plugin};

pub(crate) struct HudToolsPlugin {
    pub screenshots_dir: PathBuf,
}

impl Plugin for HudToolsPlugin {
    fn build(&self, app: &mut App) {
        screenshot::configure(app, self.screenshots_dir.clone());
    }
}
